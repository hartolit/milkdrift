use std::path::Path;

use super::orchestration::{HygieneReport, HygieneViolation};

const RULE_OPERATIONAL_INVOCATION: &str = "HYGIENE-PY-INVOKE-1";

pub(super) fn is_potential_operational_surface(path: &Path) -> bool {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();

    matches!(
        extension.to_ascii_lowercase().as_str(),
        "md" | "rs"
            | "sh"
            | "bash"
            | "zsh"
            | "fish"
            | "ps1"
            | "toml"
            | "yml"
            | "yaml"
            | "nix"
            | "slint"
    ) || matches!(file_name, "Makefile" | "Justfile" | "Dockerfile")
        || path.components().any(|component| {
            matches!(
                component.as_os_str().to_str(),
                Some("scripts" | "tools" | "build" | "release" | "packaging")
            )
        })
}

pub(super) fn scan_operational_invocations(path: &Path, content: &str, report: &mut HygieneReport) {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let matches = if extension.eq_ignore_ascii_case("md") {
        markdown_invocations(content)
    } else {
        text_surface_invocations(path, content)
    };

    for (line, command) in matches {
        report.push(HygieneViolation::new(
            Some(path.to_path_buf()),
            Some(line),
            RULE_OPERATIONAL_INVOCATION,
            format!(
                "maintained operational surface invokes prohibited `{command}` tooling; replace it with a Rust/Cargo-native command"
            ),
        ));
    }
}

fn markdown_invocations(content: &str) -> Vec<(usize, String)> {
    let mut matches = Vec::new();
    let mut in_fence = false;
    let mut negative_fence = false;
    let mut previous_nonempty = "";

    for (index, line) in content.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            if in_fence {
                in_fence = false;
                negative_fence = false;
            } else {
                in_fence = true;
                negative_fence = is_negative_policy_line(previous_nonempty);
            }
            continue;
        }

        if in_fence {
            if !negative_fence {
                collect_line_invocations(trimmed, line_number, &mut matches);
            }
        } else if !is_negative_policy_line(trimmed) && is_instructional_line(trimmed) {
            for code_span in inline_code_spans(trimmed) {
                collect_line_invocations(code_span, line_number, &mut matches);
            }
        }

        if !trimmed.is_empty() {
            previous_nonempty = trimmed;
        }
    }

    matches
}

fn text_surface_invocations(path: &Path, content: &str) -> Vec<(usize, String)> {
    let mut matches = Vec::new();
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    let shell_surface = matches!(
        extension.to_ascii_lowercase().as_str(),
        "sh" | "bash" | "zsh" | "fish" | "ps1" | "nix"
    ) || path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| matches!(name, "Makefile" | "Justfile" | "Dockerfile"));
    let config_surface = matches!(
        extension.to_ascii_lowercase().as_str(),
        "toml" | "yml" | "yaml"
    );
    let source_surface = matches!(extension.to_ascii_lowercase().as_str(), "rs" | "slint");

    for (index, line) in content.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim();
        if line_number == 1
            && trimmed.starts_with("#!")
            && let Some(command) = prohibited_shell_command(trimmed.trim_start_matches("#!"))
        {
            matches.push((line_number, command));
        }
        if shell_surface {
            collect_line_invocations(trimmed, line_number, &mut matches);
        }
        if config_surface {
            if let Some(command_line) = configured_command(trimmed) {
                collect_line_invocations(command_line, line_number, &mut matches);
            } else if trimmed.starts_with(|character: char| {
                character.is_ascii_alphabetic() || matches!(character, '$' | '/' | '.')
            }) {
                collect_line_invocations(trimmed, line_number, &mut matches);
            }
        }
        if source_surface && let Some(command) = source_command_constructor(trimmed) {
            matches.push((line_number, command));
        }
    }

    matches
}

fn collect_line_invocations(line: &str, line_number: usize, matches: &mut Vec<(usize, String)>) {
    if let Some(command_line) = configured_command(line) {
        if let Some(command) = prohibited_shell_command(command_line) {
            matches.push((line_number, command));
        }
    } else if let Some(command) = prohibited_shell_command(line) {
        matches.push((line_number, command));
    }
    if let Some(command) = source_command_constructor(line) {
        matches.push((line_number, command));
    }
}

fn configured_command(line: &str) -> Option<&str> {
    let trimmed = line.trim_start_matches(['-', ' ']).trim_start();
    for key in ["run", "command", "script", "entrypoint", "shell"] {
        let Some(remainder) = trimmed.strip_prefix(key) else {
            continue;
        };
        let remainder = remainder.trim_start();
        let Some(value) = remainder
            .strip_prefix(':')
            .or_else(|| remainder.strip_prefix('='))
        else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() || matches!(value, "|" | ">" | "|-" | ">-") {
            return None;
        }
        return Some(value.trim_matches(['\'', '"']));
    }
    None
}

fn source_command_constructor(line: &str) -> Option<String> {
    for marker in ["Command::new(", "cmd!("] {
        let Some(position) = line.find(marker) else {
            continue;
        };
        let argument = line[position + marker.len()..].trim_start();
        let Some(literal) = first_string_literal(argument) else {
            continue;
        };
        if let Some(command) = prohibited_executable(literal) {
            return Some(command);
        }
    }
    None
}

fn first_string_literal(value: &str) -> Option<&str> {
    let value = value.strip_prefix('r').unwrap_or(value);
    let quote = value.chars().next()?;
    if !matches!(quote, '\'' | '"') {
        return None;
    }
    let quoted = &value[quote.len_utf8()..];
    let end = quoted.find(quote)?;
    Some(&quoted[..end])
}

fn prohibited_shell_command(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() || (line.starts_with('#') && !line.starts_with("#!")) {
        return None;
    }

    for segment in line.split([';', '|', '&']) {
        let words = segment
            .trim()
            .trim_start_matches("#!")
            .trim_start_matches(['$', '>', ' '])
            .split_ascii_whitespace();

        for word in words {
            let cleaned = clean_shell_word(word);
            if cleaned.is_empty() || is_environment_assignment(&cleaned) {
                continue;
            }
            if matches!(
                cleaned.as_str(),
                "!" | "if"
                    | "then"
                    | "elif"
                    | "do"
                    | "sudo"
                    | "env"
                    | "command"
                    | "exec"
                    | "time"
                    | "nohup"
                    | "RUN"
            ) || cleaned.starts_with('-')
            {
                continue;
            }
            if let Some(command) = prohibited_executable(&cleaned) {
                return Some(command);
            }
            break;
        }
    }
    None
}

fn clean_shell_word(word: &str) -> String {
    let unquoted = word.trim_matches(|character: char| {
        matches!(
            character,
            '\'' | '"' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ':' | '\\'
        )
    });
    unquoted
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(unquoted)
        .to_owned()
}

fn is_environment_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn prohibited_executable(executable: &str) -> Option<String> {
    let name = clean_shell_word(executable).to_ascii_lowercase();
    let python_version = name.strip_prefix("python").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.chars().any(|character| character.is_ascii_digit())
            && suffix
                .chars()
                .all(|character| character.is_ascii_digit() || character == '.')
    });
    let pip_version = name.strip_prefix("pip").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.chars().any(|character| character.is_ascii_digit())
            && suffix
                .chars()
                .all(|character| character.is_ascii_digit() || character == '.')
    });

    if name == "python"
        || python_version
        || name == "python-config"
        || name == "pip"
        || pip_version
        || matches!(
            name.as_str(),
            "pipx" | "uv" | "conda" | "poetry" | "pytest" | "maturin" | "hf" | "huggingface-cli"
        )
    {
        Some(name)
    } else {
        None
    }
}

fn is_negative_policy_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "do not",
        "don't",
        "does not",
        "must not",
        "mustn't",
        "never",
        "forbid",
        "prohibit",
        "reject",
        "disallow",
        "not require",
        "without python",
        "no python",
    ]
    .into_iter()
    .any(|phrase| lower.contains(phrase))
}

fn is_instructional_line(line: &str) -> bool {
    [
        "run", "invoke", "execute", "install", "use", "call", "launch", "require",
    ]
    .into_iter()
    .any(|word| contains_ascii_word(line, word))
}

fn contains_ascii_word(value: &str, word: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.match_indices(word).any(|(position, _)| {
        let before = lower[..position].chars().next_back();
        let after = lower[position + word.len()..].chars().next();
        before.is_none_or(|character| !is_word_character(character))
            && after.is_none_or(|character| !is_word_character(character))
    })
}

const fn is_word_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

fn inline_code_spans(line: &str) -> Vec<&str> {
    line.split('`')
        .enumerate()
        .filter_map(|(index, part)| (index % 2 == 1 && !part.is_empty()).then_some(part))
        .collect()
}
