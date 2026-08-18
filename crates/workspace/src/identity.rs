use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::WorkspaceError;

const MAX_STANDARD_ID_BYTES: usize = 128;
const MAX_EXTENDED_ID_BYTES: usize = 192;

fn validate_identity(
    value: &str,
    type_name: &'static str,
    maximum_bytes: usize,
) -> Result<(), WorkspaceError> {
    if value.is_empty() || value.len() > maximum_bytes {
        return Err(WorkspaceError::InvalidIdentity {
            type_name,
            reason: format!("length must be between 1 and {maximum_bytes} bytes"),
        });
    }
    if !value.is_ascii() {
        return Err(WorkspaceError::InvalidIdentity {
            type_name,
            reason: "must contain ASCII characters only".to_owned(),
        });
    }
    if !value.as_bytes()[0].is_ascii_alphanumeric()
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
    {
        return Err(WorkspaceError::InvalidIdentity {
            type_name,
            reason: "must start with an alphanumeric character and use only alphanumerics, '-', '_', '.', ':', or '/'"
                .to_owned(),
        });
    }
    Ok(())
}

macro_rules! identity_type {
    ($(#[$meta:meta])* $name:ident, $maximum:expr) => {
        $(#[$meta])*
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            /// Constructs and validates the identity.
            pub fn new(value: impl Into<String>) -> Result<Self, WorkspaceError> {
                let value = value.into();
                validate_identity(&value, stringify!($name), $maximum)?;
                Ok(Self(value))
            }

            /// Returns the validated identity text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = WorkspaceError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::new(value)
            }
        }

        impl TryFrom<String> for $name {
            type Error = WorkspaceError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

identity_type!(
    /// Stable identity of one durable run aggregate.
    RunId,
    MAX_STANDARD_ID_BYTES
);
identity_type!(
    /// Stable identity of one workspace scope within a run.
    ScopeId,
    MAX_STANDARD_ID_BYTES
);
identity_type!(
    /// Stable semantic identity of one structured fork branch.
    BranchId,
    MAX_STANDARD_ID_BYTES
);
identity_type!(
    /// Stable identity of one repeat iteration.
    IterationId,
    MAX_STANDARD_ID_BYTES
);
identity_type!(
    /// Stable identity of one pinned child-subworkflow execution.
    SubworkflowId,
    MAX_STANDARD_ID_BYTES
);
identity_type!(
    /// Stable key for an immutable value stream inside one scope.
    ValueKey,
    MAX_EXTENDED_ID_BYTES
);
identity_type!(
    /// Stable logical identity of an artifact metadata record.
    ArtifactId,
    MAX_EXTENDED_ID_BYTES
);
identity_type!(
    /// Bounded opaque identity for an external causal source.
    CausalId,
    MAX_EXTENDED_ID_BYTES
);

/// Monotonically increasing version of one scope-local value stream.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ValueVersion(u64);

impl ValueVersion {
    /// First valid version of a value stream.
    pub const FIRST: Self = Self(1);

    /// Constructs a non-zero value version.
    pub fn new(value: u64) -> Result<Self, WorkspaceError> {
        if value == 0 {
            return Err(WorkspaceError::InvalidValue(
                "value version must be greater than zero".to_owned(),
            ));
        }
        Ok(Self(value))
    }

    /// Returns the numeric version.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns the next version, failing instead of wrapping at `u64::MAX`.
    pub fn next(self) -> Result<Self, WorkspaceError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(WorkspaceError::AccountingOverflow("value version"))
    }
}

impl<'de> Deserialize<'de> for ValueVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u64::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for ValueVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[cfg(test)]
mod tests {
    use super::ValueVersion;

    #[test]
    fn value_versions_are_nonzero_and_never_wrap() {
        assert!(ValueVersion::new(0).is_err());
        assert!(matches!(
            ValueVersion::FIRST.next().map(ValueVersion::get),
            Ok(2)
        ));
        assert!(
            ValueVersion::new(u64::MAX)
                .and_then(ValueVersion::next)
                .is_err()
        );
    }
}
