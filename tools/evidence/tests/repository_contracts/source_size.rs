//! Measure implementation growth without charging comments against the same allowance.

use std::collections::BTreeSet;

use proc_macro2::{Delimiter, Group, Span, TokenStream, TokenTree};

use super::TestResult;

pub(super) struct SourceSize {
    pub(super) physical: usize,
    pub(super) implementation: usize,
}

pub(super) fn source_size(source: &str) -> TestResult<SourceSize> {
    // Tokenization distinguishes comments from the same text inside a Rust literal.
    // It also represents rustdoc comments as doc attributes, handled with explicit ones below.
    let tokens: TokenStream = source.parse()?;
    let mut lines = BTreeSet::new();
    collect_code_lines(tokens, &mut lines);
    Ok(SourceSize {
        physical: source.lines().count(),
        implementation: lines.len(),
    })
}

fn collect_code_lines(stream: TokenStream, lines: &mut BTreeSet<usize>) {
    let tokens: Vec<_> = stream.into_iter().collect();
    let mut position = 0;
    while position < tokens.len() {
        let documentation = match &tokens[position..] {
            [TokenTree::Punct(hash), TokenTree::Group(attribute), ..]
                if hash.as_char() == '#' && is_documentation(attribute) =>
            {
                Some(2)
            }
            [
                TokenTree::Punct(hash),
                TokenTree::Punct(bang),
                TokenTree::Group(attribute),
                ..,
            ] if hash.as_char() == '#' && bang.as_char() == '!' && is_documentation(attribute) => {
                Some(3)
            }
            _ => None,
        };
        if let Some(length) = documentation {
            position += length;
            continue;
        }
        match &tokens[position] {
            TokenTree::Group(group) => {
                // A group's full span includes the comments between its delimiters.
                record_span(group.span_open(), lines);
                collect_code_lines(group.stream(), lines);
                record_span(group.span_close(), lines);
            }
            token => record_span(token.span(), lines),
        }
        position += 1;
    }
}

fn is_documentation(group: &Group) -> bool {
    group.delimiter() == Delimiter::Bracket
        && matches!(group.stream().into_iter().next(), Some(TokenTree::Ident(name)) if name == "doc")
}

fn record_span(span: Span, lines: &mut BTreeSet<usize>) {
    lines.extend(span.start().line..=span.end().line);
}

#[test]
fn explanations_do_not_consume_the_implementation_allowance() -> TestResult {
    let code = "pub fn run() {\n    call();\n}\n";
    let documented = format!(
        "//! Module purpose.\n\n{}\n/** An outer block explanation. */\n\
         #[doc = \"An explicit\nmultiline explanation.\"]\n\
         pub fn run() {{\n\
         /*! Inner documentation. */\n\
         // A decision made by the implementation.\n\
         /* Ordinary block with /* a nested comment */ inside. */\n\
         call(); // A trailing explanation.\n\
         }}\n",
        "/// More explanation.\n".repeat(super::MAXIMUM_SOURCE_LINES)
    );
    assert_eq!(source_size(code)?.implementation, 3);
    let size = source_size(&documented)?;
    assert!(size.physical > super::MAXIMUM_SOURCE_LINES);
    assert_eq!(size.implementation, source_size(code)?.implementation);
    assert_eq!(
        source_size(&documented.replace('\n', "\r\n"))?.implementation,
        size.implementation
    );
    assert_eq!(source_size("/* Only a comment. */\n\n")?.implementation, 0);
    Ok(())
}

#[test]
fn literal_contents_and_non_documentation_attributes_still_count() -> TestResult {
    let source = r###"#[cfg(test)]
fn run() { /** Documentation beside code. */
    let text = r#"
/// This belongs to the string.
#[doc = "This also belongs to the string."]
/* Not a block comment. */
"#;
    let slash = '/';
    let escaped = "\"// still inside a string";
}
"###;
    assert_eq!(source_size(source)?.implementation, 10);
    assert!(source_size("fn broken( {").is_err());
    Ok(())
}

#[test]
fn code_growth_still_requires_a_cohesion_review() -> TestResult {
    let code = format!(
        "fn run() {{\n{}}}\n",
        "    call();\n".repeat(super::COHESION_REVIEW_LINES - 2)
    );
    let at_limit = source_size(&code)?;
    assert_eq!(at_limit.implementation, super::COHESION_REVIEW_LINES);
    let reviewed =
        super::SourceLineCount::production("crates/example/src/run.rs", at_limit.implementation);
    assert!(super::cohesion_policy_errors(&[], &[reviewed]).is_empty());

    let grown = source_size(&format!("{code}const MORE: bool = true;\n"))?;
    let unreviewed =
        super::SourceLineCount::production("crates/example/src/run.rs", grown.implementation);
    assert!(
        super::cohesion_policy_errors(&[], &[unreviewed])
            .iter()
            .any(|error| error.contains("missing exception"))
    );
    Ok(())
}
