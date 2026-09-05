//! Parse maintained command examples with the same Clap owner as the binary.
use super::Cli;
use clap::Parser;
use std::{
    fs,
    path::{Path, PathBuf},
};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn collect_markdown(directory: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != "decisions") {
                collect_markdown(&path, files)?;
            }
        } else if path.extension().is_some_and(|ext| ext == "md") {
            files.push(path);
        }
    }
    Ok(())
}

// Tokenize only the documented literal argument subset. Do not evaluate shell expressions,
// expand environment variables, launch commands, or read the files passed as arguments.
fn arguments(command: &str) -> TestResult<Vec<String>> {
    let mut result = vec!["milkdrift".to_owned()];
    let mut token = String::new();
    let mut quote = None;
    for character in command.chars() {
        if let Some(opened) = quote {
            if character == opened {
                quote = None;
            } else {
                token.push(character);
            }
        } else {
            match character {
                '\'' | '"' => quote = Some(character),
                '|' | '>' => break,
                ch if ch.is_whitespace() => {
                    if !token.is_empty() {
                        result.push(std::mem::take(&mut token));
                    }
                }
                ch => token.push(ch),
            }
        }
    }
    if quote.is_some() {
        return Err("unclosed shell quote in documentation".into());
    }
    if !token.is_empty() {
        result.push(token);
    }
    for index in 1..result.len() {
        // Only observed numeric sequence placeholders are replaced; unknown flags and values
        // still reach the real parser and fail. Identity/path placeholders remain ordinary strings.
        if result[index - 1] == "--expected-sequence"
            && result[index]
                .chars()
                .all(|ch| ch.is_ascii_uppercase() || ch == '_')
        {
            result[index] = "1".to_owned();
        }
    }
    Ok(result)
}

fn commands(document: &str) -> TestResult<Vec<Vec<String>>> {
    let mut shell = false;
    let mut pending = String::new();
    let mut found = Vec::new();
    for line in document.lines() {
        let line = line.trim();
        if line.starts_with("```") {
            if !pending.is_empty() {
                return Err("unfinished command continuation".into());
            }
            shell = matches!(line, "```sh" | "```bash" | "```powershell");
            continue;
        }
        if !shell {
            continue;
        }
        let command = if !pending.is_empty() {
            Some(line)
        } else if let Some(tail) = line.strip_prefix("milkdrift ") {
            Some(tail)
        } else if let Some((_, tail)) = line.split_once("& $cli ") {
            Some(tail)
        } else {
            line.strip_prefix("cargo run -p milkdrift-cli -- ")
        };
        let Some(command) = command else {
            continue;
        };
        let continued = command.ends_with('\\') || command.ends_with('`');
        pending.push_str(command.trim_end_matches(['\\', '`']));
        if continued {
            pending.push(' ');
        } else {
            found.push(arguments(&pending)?);
            pending.clear();
        }
    }
    if !pending.is_empty() {
        return Err("unfinished command continuation".into());
    }
    Ok(found)
}

#[test]
fn maintained_cli_command_examples_parse_and_bound_waits() -> TestResult {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut documents = vec![repository.join("README.md")];
    collect_markdown(&repository.join("docs"), &mut documents)?;
    collect_markdown(&repository.join("examples"), &mut documents)?;
    let mut total = 0;
    for path in documents {
        let examples = commands(&fs::read_to_string(&path)?)?;
        if path == repository.join("README.md") {
            assert!(!examples.is_empty(), "quick start needs CLI commands");
        }
        for args in examples {
            let cli = Cli::try_parse_from(&args)
                .map_err(|error| format!("{}: {args:?}: {error}", path.display()))?;
            assert!(
                !(cli.is_wait() || cli.is_follow()) || cli.timeout_secs.is_some(),
                "{}: documented wait/follow has no deadline: {args:?}",
                path.display()
            );
            total += 1;
        }
    }
    assert!(total >= 30, "documented operator routes were not inspected");
    Ok(())
}

#[test]
fn documentation_parser_preserves_quoted_values_and_detects_obsolete_flags() -> TestResult {
    let docs = "```sh\nmilkdrift --reason 'two words' \\\n run show RUN_ID | consumer\nmilkdrift run show RUN_ID --obsolete-option\n```\n";
    let examples = commands(docs)?;
    assert_eq!(examples.len(), 2);
    let cli = Cli::try_parse_from(&examples[0])?;
    assert_eq!(cli.reason, "two words");
    assert!(Cli::try_parse_from(&examples[1]).is_err());
    assert!(arguments("--reason 'unterminated").is_err());
    Ok(())
}
