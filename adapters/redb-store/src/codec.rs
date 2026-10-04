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

pub(crate) fn decode_components<const N: usize>(
    encoded: &[u8],
) -> Result<[&str; N], PersistenceError> {
    let mut offset = 0_usize;
    let mut decoded = [""; N];
    for component in &mut decoded {
        let length_end = offset
            .checked_add(COMPONENT_LENGTH_BYTES)
            .ok_or_else(|| bounds("compound key offset overflow"))?;
        let length_bytes: [u8; COMPONENT_LENGTH_BYTES] = encoded
            .get(offset..length_end)
            .ok_or_else(|| bounds("compound key has a truncated component length"))?
            .try_into()
            .map_err(|_| bounds("compound key component length is malformed"))?;
        let length = usize::try_from(u32::from_be_bytes(length_bytes))
            .map_err(|_| bounds("compound key component length cannot be represented"))?;
        let value_end = length_end
            .checked_add(length)
            .ok_or_else(|| bounds("compound key component end overflow"))?;
        let value = encoded
            .get(length_end..value_end)
            .ok_or_else(|| bounds("compound key has a truncated component"))?;
        *component = std::str::from_utf8(value).map_err(|_| {
            PersistenceError::Corruption("compound key component is not valid UTF-8".to_owned())
        })?;
        offset = value_end;
    }
    if offset != encoded.len() {
        return Err(bounds("compound key contains trailing bytes"));
    }
    Ok(decoded)
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

    #[test]
    fn compound_components_round_trip_exactly() -> Result<(), PersistenceError> {
        let encoded = components(&["first", "second", "third"])?;
        assert_eq!(
            decode_components::<3>(&encoded)?,
            ["first", "second", "third"]
        );
        assert!(decode_components::<2>(&encoded).is_err());
        assert!(decode_components::<4>(&encoded).is_err());
        for length in 0..encoded.len() {
            let truncated = encoded
                .get(..length)
                .ok_or_else(|| bounds("test prefix missing"))?;
            assert!(decode_components::<3>(truncated).is_err());
        }
        let invalid_utf8 = [0, 0, 0, 1, 0xff];
        assert!(decode_components::<1>(&invalid_utf8).is_err());
        let empty = components(&["", "", ""])?;
        assert_eq!(decode_components::<3>(&empty)?, ["", "", ""]);
        assert!(decode_components::<0>(&[])?.is_empty());
        Ok(())
    }
}
