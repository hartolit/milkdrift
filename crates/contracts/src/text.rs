/// Returns the longest prefix of `value` whose UTF-8 byte length does not exceed
/// `maximum_bytes`.
///
/// The returned slice borrows the input and therefore performs no allocation.
#[must_use]
pub fn truncate_utf8(value: &str, maximum_bytes: usize) -> &str {
    if value.len() <= maximum_bytes {
        return value;
    }
    let mut boundary = maximum_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &value[..boundary]
}

/// Whether `value` is exactly `b3_` followed by 64 lowercase hexadecimal bytes.
///
/// This checks lexical form only. Digest domains, hashing policy, semantic types,
/// and error vocabularies remain with their owning packages.
#[must_use]
pub fn is_canonical_blake3_digest(value: &str) -> bool {
    value.strip_prefix("b3_").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

#[cfg(test)]
mod tests {
    use super::{is_canonical_blake3_digest, truncate_utf8};

    #[test]
    fn utf8_truncation_obeys_byte_and_character_boundaries() {
        for (value, maximum, expected) in [
            ("hello", 0, ""),
            ("hello", 5, "hello"),
            ("hello", 4, "hell"),
            ("éclair", 1, ""),
            ("éclair", 2, "é"),
            ("éclair", 3, "éc"),
            ("short", 64, "short"),
        ] {
            assert_eq!(truncate_utf8(value, maximum), expected);
        }
    }

    #[test]
    fn canonical_blake3_lexical_form_is_exact() {
        let valid = format!("b3_{}", "a".repeat(64));
        assert!(is_canonical_blake3_digest(&valid));

        for invalid in [
            String::new(),
            format!("sha3_{}", "a".repeat(64)),
            format!("b3_{}", "a".repeat(63)),
            format!("b3_{}", "a".repeat(65)),
            format!("b3_{}A", "a".repeat(63)),
            format!("b3_{}g", "a".repeat(63)),
            format!("b3_{}é", "a".repeat(62)),
        ] {
            assert!(!is_canonical_blake3_digest(&invalid), "accepted {invalid}");
        }
    }
}
