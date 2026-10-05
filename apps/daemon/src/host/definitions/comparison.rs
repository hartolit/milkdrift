//! Compare semantic fields lazily so a full response never requires retaining the omitted diff.

use milkdrift_blueprint::SemanticBlueprint;
use milkdrift_control_protocol::RevisionChange;
use std::{cmp::Ordering, collections::BTreeMap};

fn change<T: PartialEq + ?Sized>(
    subject: &str,
    identity: Option<String>,
    left: Option<&T>,
    right: Option<&T>,
) -> Option<RevisionChange> {
    let change = match (left, right) {
        (None, Some(_)) => "added",
        (Some(_), None) => "removed",
        (Some(left), Some(right)) if left != right => "changed",
        _ => return None,
    };
    Some(RevisionChange {
        subject: subject.to_owned(),
        identity,
        change: change.to_owned(),
        // Values may contain prompts or private semantic extensions. Identity and category
        // locate the difference without copying those values into a summary.
        detail: serde_json::Value::Null,
    })
}

fn map_changes<'a, K: Ord + ToString, V: PartialEq>(
    subject: &'a str,
    left: &'a BTreeMap<K, V>,
    right: &'a BTreeMap<K, V>,
) -> impl Iterator<Item = RevisionChange> + 'a {
    let mut left = left.iter().peekable();
    let mut right = right.iter().peekable();
    std::iter::from_fn(move || {
        loop {
            let order = match (left.peek(), right.peek()) {
                (None, None) => return None,
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (Some((a, _)), Some((b, _))) => a.cmp(b),
            };
            let (key, before, after) = match order {
                Ordering::Less => {
                    let (key, value) = left.next()?;
                    (key, Some(value), None)
                }
                Ordering::Greater => {
                    let (key, value) = right.next()?;
                    (key, None, Some(value))
                }
                Ordering::Equal => {
                    let (key, before) = left.next()?;
                    let (_, after) = right.next()?;
                    (key, Some(before), Some(after))
                }
            };
            if let Some(value) = change(subject, Some(key.to_string()), before, after) {
                return Some(value);
            }
        }
    })
}

pub(super) fn changes<'a>(
    left: &'a SemanticBlueprint,
    right: &'a SemanticBlueprint,
) -> impl Iterator<Item = RevisionChange> + 'a {
    let a = left.metadata();
    let b = right.metadata();
    // Workflow identity is checked by the authorized caller. Blueprint identity is a separate
    // saved semantic field, even though ordinary genesis currently derives it from the workflow.
    change(
        "blueprint",
        None,
        Some(left.blueprint()),
        Some(right.blueprint()),
    )
    .into_iter()
    .chain(change(
        "metadata",
        Some("name".into()),
        Some(a.name()),
        Some(b.name()),
    ))
    .chain(change(
        "metadata",
        Some("description".into()),
        Some(a.description()),
        Some(b.description()),
    ))
    .chain(change(
        "metadata",
        Some("labels".into()),
        Some(a.labels()),
        Some(b.labels()),
    ))
    .chain(map_changes("extension", a.extensions(), b.extensions()))
    .chain(map_changes(
        "input",
        left.interface().inputs(),
        right.interface().inputs(),
    ))
    .chain(map_changes(
        "output",
        left.interface().outputs(),
        right.interface().outputs(),
    ))
    .chain(change(
        "agreement",
        None,
        left.agreement(),
        right.agreement(),
    ))
    .chain(map_changes("node", left.nodes(), right.nodes()))
    .chain(map_changes("edge", left.edges(), right.edges()))
}
