use milkdrift_persistence::{PersistenceError, RunSequence};

const COMPONENT_LENGTH_BYTES: usize = 4;

pub(crate) fn component(value: &str) -> Result<Vec<u8>, PersistenceError> {
    let mut encoded = Vec::with_capacity(
        COMPONENT_LENGTH_BYTES
            .checked_add(value.len())
            .ok_or_else(|| bounds("key component length overflow"))?,
    );
    push_component(&mut encoded, value)?;
    Ok(encoded)
}

pub(crate) fn pair(first: &str, second: &str) -> Result<Vec<u8>, PersistenceError> {
    let mut encoded = Vec::with_capacity(
        COMPONENT_LENGTH_BYTES
            .checked_mul(2)
            .and_then(|prefix| prefix.checked_add(first.len()))
            .and_then(|length| length.checked_add(second.len()))
            .ok_or_else(|| bounds("compound key length overflow"))?,
    );
    push_component(&mut encoded, first)?;
    push_component(&mut encoded, second)?;
    Ok(encoded)
}

pub(crate) fn triple(first: &str, second: &str, third: &str) -> Result<Vec<u8>, PersistenceError> {
    let mut encoded = pair(first, second)?;
    push_component(&mut encoded, third)?;
    Ok(encoded)
}

pub(crate) fn components(values: &[&str]) -> Result<Vec<u8>, PersistenceError> {
    let mut encoded = Vec::new();
    for value in values {
        push_component(&mut encoded, value)?;
    }
    Ok(encoded)
}

pub(crate) fn run_sequence(run: &str, sequence: RunSequence) -> Result<Vec<u8>, PersistenceError> {
    let mut encoded = Vec::with_capacity(
        COMPONENT_LENGTH_BYTES
            .checked_add(run.len())
            .and_then(|length| length.checked_add(size_of::<u64>()))
            .ok_or_else(|| bounds("event key length overflow"))?,
    );
    push_component(&mut encoded, run)?;
    encoded.extend_from_slice(&sequence.get().to_be_bytes());
    Ok(encoded)
}

pub(crate) fn value(
    run: &str,
    scope: &str,
    key: &str,
    version: u64,
) -> Result<Vec<u8>, PersistenceError> {
    let mut encoded = triple(run, scope, key)?;
    encoded.extend_from_slice(&version.to_be_bytes());
    Ok(encoded)
}

pub(crate) fn value_prefix(run: &str, scope: &str, key: &str) -> Result<Vec<u8>, PersistenceError> {
    triple(run, scope, key)
}

pub(crate) fn prefix_end(mut prefix: Vec<u8>) -> Option<Vec<u8>> {
    for byte in prefix.iter_mut().rev() {
        if *byte != u8::MAX {
            *byte += 1;
            return Some(prefix);
        }
        *byte = 0;
    }
    None
}

pub(crate) fn ordered_timestamp(
    timestamp_millis: u64,
    identity: &str,
) -> Result<Vec<u8>, PersistenceError> {
    let mut encoded = Vec::with_capacity(
        size_of::<u64>()
            .checked_add(COMPONENT_LENGTH_BYTES)
            .and_then(|length| length.checked_add(identity.len()))
            .ok_or_else(|| bounds("ordered index key length overflow"))?,
    );
    encoded.extend_from_slice(&timestamp_millis.to_be_bytes());
    push_component(&mut encoded, identity)?;
    Ok(encoded)
}

fn push_component(encoded: &mut Vec<u8>, component: &str) -> Result<(), PersistenceError> {
    let length = u32::try_from(component.len()).map_err(|_| bounds("key component exceeds u32"))?;
    encoded.extend_from_slice(&length.to_be_bytes());
    encoded.extend_from_slice(component.as_bytes());
    Ok(())
}

fn bounds(reason: &str) -> PersistenceError {
    PersistenceError::Bounds {
        location: "storage_key",
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn length_prefixes_prevent_component_ambiguity() -> Result<(), PersistenceError> {
        let first = pair("a", "bc")?;
        let second = pair("ab", "c")?;
        assert_ne!(first, second);
        Ok(())
    }

    #[test]
    fn big_endian_sequence_preserves_numeric_order() -> Result<(), PersistenceError> {
        let lower = run_sequence("run", RunSequence::new(255))?;
        let higher = run_sequence("run", RunSequence::new(256))?;
        assert!(lower < higher);
        Ok(())
    }
}
