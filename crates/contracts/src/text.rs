/// Returns the longest prefix of `value` whose UTF-8 byte length does not exceed
/// `maximum_bytes`.
///
/// The returned slice borrows the input and therefore performs no allocation.
/// Callers use this to fit text, such as a diagnostic summary, into their own byte limit.
/// Zero returns an empty slice; a limit at least as large as the input returns it whole.
/// A limit inside a multibyte character backs up to that character's start. This preserves
/// UTF-8, but can split a user-perceived character made from multiple Unicode scalars.
/// It adds no ellipsis and performs no redaction; the caller owns those choices.
///
/// ```
/// use milkdrift_contracts::truncate_utf8;
///
/// assert_eq!(truncate_utf8("éclair", 1), "");
/// assert_eq!(truncate_utf8("éclair", 3), "éc");
/// assert_eq!(truncate_utf8("hello", 0), "");
/// assert_eq!(truncate_utf8("hello", 64), "hello");
/// ```
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
/// Use it when reading digest text; the owner must separately compute and compare a
/// digest to verify referenced content. The prefix and hex letters are case-sensitive,
/// whitespace is refused, and no normalization is performed.
///
/// ```
/// use milkdrift_contracts::is_canonical_blake3_digest;
///
/// let text = format!("b3_{}", "a".repeat(64));
/// assert!(is_canonical_blake3_digest(&text));
/// assert!(!is_canonical_blake3_digest(&text.to_uppercase()));
/// assert!(!is_canonical_blake3_digest(&format!("{text}\n")));
/// ```
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
