use std::{collections::BTreeSet, fmt, marker::PhantomData};

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error as _, IgnoredAny, MapAccess, SeqAccess, Visitor},
    ser::SerializeStruct,
};

use crate::AuthorityError;

/// Maximum number of exact values in one authority selector.
pub const MAX_SELECTION_ITEMS: usize = 128;

/// Explicit selection of either every value or a nonempty bounded exact allowlist.
///
/// `Any` is represented explicitly on the wire. Exact values are kept in canonical order and
/// cannot be mutated through this API, so an empty allowlist can never acquire wildcard meaning.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Selection<T> {
    kind: SelectionKind<T>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum SelectionKind<T> {
    Any,
    Only(BTreeSet<T>),
}

impl<T> Selection<T> {
    /// Selects every value explicitly.
    #[must_use]
    pub const fn any() -> Self {
        Self {
            kind: SelectionKind::Any,
        }
    }

    /// Selects one nonempty bounded set of exact values.
    pub fn only(values: BTreeSet<T>) -> Result<Self, AuthorityError> {
        validate_count(values.len())?;
        Ok(Self {
            kind: SelectionKind::Only(values),
        })
    }

    /// Selects one exact value.
    #[must_use]
    pub fn only_one(value: T) -> Self
    where
        T: Ord,
    {
        Self {
            kind: SelectionKind::Only(BTreeSet::from([value])),
        }
    }

    /// Whether this selector is the explicit wildcard.
    #[must_use]
    pub const fn is_any(&self) -> bool {
        matches!(self.kind, SelectionKind::Any)
    }

    /// Returns exact values for `Only`, and `None` for `Any`.
    #[must_use]
    pub const fn only_values(&self) -> Option<&BTreeSet<T>> {
        match &self.kind {
            SelectionKind::Any => None,
            SelectionKind::Only(values) => Some(values),
        }
    }

    /// Tests one value against this selector.
    #[must_use]
    pub fn matches(&self, value: &T) -> bool
    where
        T: Ord,
    {
        match &self.kind {
            SelectionKind::Any => true,
            SelectionKind::Only(values) => values.contains(value),
        }
    }

    /// Tests whether every value admitted here is also admitted by `other`.
    #[must_use]
    pub fn is_subset_of(&self, other: &Self) -> bool
    where
        T: Ord,
    {
        match (&self.kind, &other.kind) {
            (SelectionKind::Any, SelectionKind::Any)
            | (SelectionKind::Only(_), SelectionKind::Any) => true,
            (SelectionKind::Any, SelectionKind::Only(_)) => false,
            (SelectionKind::Only(requested), SelectionKind::Only(allowed)) => {
                requested.is_subset(allowed)
            }
        }
    }
}

impl<T: Serialize> Serialize for Selection<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match &self.kind {
            SelectionKind::Any => {
                let mut state = serializer.serialize_struct("Selection", 1)?;
                state.serialize_field("type", "any")?;
                state.end()
            }
            SelectionKind::Only(values) => {
                let mut state = serializer.serialize_struct("Selection", 2)?;
                state.serialize_field("type", "only")?;
                state.serialize_field("values", values)?;
                state.end()
            }
        }
    }
}

impl<'de, T> Deserialize<'de> for Selection<T>
where
    T: Deserialize<'de> + Ord,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(SelectionVisitor(PhantomData))
    }
}

struct SelectionVisitor<T>(PhantomData<T>);

impl<'de, T> Visitor<'de> for SelectionVisitor<T>
where
    T: Deserialize<'de> + Ord,
{
    type Value = Selection<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an explicit any or nonempty only authority selector")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut selector_type = None;
        let mut values = None;
        while let Some(field) = map.next_key::<Field>()? {
            match field {
                Field::Type => {
                    if selector_type.is_some() {
                        return Err(A::Error::duplicate_field("type"));
                    }
                    selector_type = Some(map.next_value::<SelectorType>()?);
                }
                Field::Values => {
                    if values.is_some() {
                        return Err(A::Error::duplicate_field("values"));
                    }
                    values = Some(map.next_value::<BoundedValues<T>>()?.0);
                }
                Field::Unknown(name) => {
                    let _: IgnoredAny = map.next_value()?;
                    return Err(A::Error::unknown_field(&name, &["type", "values"]));
                }
            }
        }
        match selector_type.ok_or_else(|| A::Error::missing_field("type"))? {
            SelectorType::Any => {
                if values.is_some() {
                    return Err(A::Error::custom("any selector must not contain values"));
                }
                Ok(Selection::any())
            }
            SelectorType::Only => {
                Selection::only(values.ok_or_else(|| A::Error::missing_field("values"))?)
                    .map_err(A::Error::custom)
            }
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum SelectorType {
    Any,
    Only,
}

enum Field {
    Type,
    Values,
    Unknown(String),
}

impl<'de> Deserialize<'de> for Field {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct FieldVisitor;

        impl Visitor<'_> for FieldVisitor {
            type Value = Field;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a selector field")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(match value {
                    "type" => Field::Type,
                    "values" => Field::Values,
                    value => Field::Unknown(value.to_owned()),
                })
            }
        }

        deserializer.deserialize_identifier(FieldVisitor)
    }
}

struct BoundedValues<T>(BTreeSet<T>);

impl<'de, T> Deserialize<'de> for BoundedValues<T>
where
    T: Deserialize<'de> + Ord,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(BoundedValuesVisitor(PhantomData))
    }
}

struct BoundedValuesVisitor<T>(PhantomData<T>);

impl<'de, T> Visitor<'de> for BoundedValuesVisitor<T>
where
    T: Deserialize<'de> + Ord,
{
    type Value = BoundedValues<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "1..={MAX_SELECTION_ITEMS} ordered unique selector values"
        )
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        if sequence
            .size_hint()
            .is_some_and(|size| size > MAX_SELECTION_ITEMS)
        {
            return Err(A::Error::invalid_length(
                sequence.size_hint().unwrap_or(MAX_SELECTION_ITEMS + 1),
                &self,
            ));
        }
        let mut values = BTreeSet::new();
        let mut count = 0;
        while let Some(value) = sequence.next_element()? {
            count += 1;
            if count > MAX_SELECTION_ITEMS {
                return Err(A::Error::invalid_length(count, &self));
            }
            if !values.insert(value) {
                return Err(A::Error::custom(
                    "only selector values must be unique in the wire document",
                ));
            }
        }
        validate_count(values.len()).map_err(A::Error::custom)?;
        Ok(BoundedValues(values))
    }
}

fn validate_count(count: usize) -> Result<(), AuthorityError> {
    if (1..=MAX_SELECTION_ITEMS).contains(&count) {
        Ok(())
    } else {
        Err(AuthorityError::Bounds {
            location: "authority.selection",
            reason: format!("only selector requires 1..={MAX_SELECTION_ITEMS} unique values"),
        })
    }
}
