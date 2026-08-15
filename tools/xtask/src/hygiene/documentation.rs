use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::orchestration::{HygieneError, HygieneReport, HygieneViolation};

const RULE_DOCUMENTATION_AUTHORITY: &str = "HYGIENE-DOCUMENTATION-AUTHORITY-1";
const IMPLEMENTATION_STATUS: &str = "docs/project/implementation-status.md";
const VALIDATION: &str = "docs/project/validation.md";
const PERFORMANCE: &str = "docs/project/performance.md";
const HISTORY: &str = "docs/agent/execution/history.md";

const CURRENT_STATE_OWNERS: &[&str] = &[
    "README.md",
    IMPLEMENTATION_STATUS,
    "docs/agent/execution/current.md",
    "docs/agent/execution/execution-plan.md",
];

pub(super) fn scan_documentation_authority(
    root: &Path,
    present: &BTreeSet<PathBuf>,
    report: &mut HygieneReport,
) -> Result<(), HygieneError> {
    if !present.contains(Path::new("docs/README.md")) {
        return Ok(());
    }

    for owner in CURRENT_STATE_OWNERS {
        let path = Path::new(owner);
        if !present.contains(path) {
            continue;
        }
        let content = read_document(root, path)?;
        scan_mutable_current_checkout_claims(path, &content, report);
    }

    let status_path = Path::new(IMPLEMENTATION_STATUS);
    if present.contains(status_path) {
        let content = read_document(root, status_path)?;
        scan_historical_evidence_rows(status_path, &content, report);
    }

    for path in present.iter().filter(|path| is_current_reference(path)) {
        let content = read_document(root, path)?;
        scan_duplicate_evidence_authority(path, &content, report);
    }

    let plan_path = Path::new("docs/agent/execution/execution-plan.md");
    if present.contains(plan_path) {
        let content = read_document(root, plan_path)?;
        scan_plan_rows(plan_path, &content, report);
    }

    Ok(())
}

fn read_document(root: &Path, path: &Path) -> Result<String, HygieneError> {
    fs::read_to_string(root.join(path)).map_err(|error| {
        HygieneError::new(format!(
            "could not read current documentation authority {}: {error}",
            path.display()
        ))
    })
}

fn scan_mutable_current_checkout_claims(path: &Path, content: &str, report: &mut HygieneReport) {
    for (line, block) in markdown_blocks(content) {
        let lower = block.to_ascii_lowercase();
        if !contains_current_checkout_subject(&lower) || !contains_remote_evidence_subject(&lower) {
            continue;
        }

        if contains_any(
            &lower,
            &[
                "has not run",
                "have not run",
                "not run remotely",
                "has no exact",
                "has no remote",
                "has no hosted",
                "has no cuda",
                "no exact hosted",
                "no exact cuda",
                "no remote result",
                "no hosted result",
                "no cuda result",
                "result does not exist",
                "results do not exist",
                "pending",
                "waiting",
                "unrun",
                "not been pushed",
                "waiting to be pushed",
            ],
        ) {
            report.push(HygieneViolation::new(
                Some(path.to_path_buf()),
                Some(line),
                RULE_DOCUMENTATION_AUTHORITY,
                "current-state documentation must not predict that its own checkout is pending, unrun, or waiting for a push; determine exact-commit remote acceptance externally"
                    .to_owned(),
            ));
        }

        let mutable_run_outcome = contains_run_reference(&lower)
            && contains_any(
                &lower,
                &["passed", "success", "succeeded", "accepted", "green"],
            );
        if mutable_run_outcome
            || contains_any(
                &lower,
                &[
                    " has passed",
                    " succeeded",
                    " is accepted",
                    " was accepted",
                    " is green",
                    " has succeeded",
                ],
            )
        {
            report.push(HygieneViolation::new(
                Some(path.to_path_buf()),
                Some(line),
                RULE_DOCUMENTATION_AUTHORITY,
                "current-state documentation must not store a mutable claim that its own checkout passed remote acceptance; record exact historical commit/tree evidence or evaluate the checkout externally"
                    .to_owned(),
            ));
        }
    }
}

fn scan_historical_evidence_rows(path: &Path, content: &str, report: &mut HygieneReport) {
    let mut section = "";
    for (index, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") {
            section = trimmed;
        }
        let lower = trimmed.to_ascii_lowercase();

        if lower.contains("latest accepted") || lower.contains("current-tree consequence") {
            report.push(HygieneViolation::new(
                Some(path.to_path_buf()),
                Some(index + 1),
                RULE_DOCUMENTATION_AUTHORITY,
                "implementation status must describe accepted runs as historical exact-tree evidence, not as a latest/current-tree boundary"
                    .to_owned(),
            ));
        }

        if !has_actions_run_link(&lower) {
            continue;
        }
        if section != "## Accepted historical evidence" {
            report.push(HygieneViolation::new(
                Some(path.to_path_buf()),
                Some(index + 1),
                RULE_DOCUMENTATION_AUTHORITY,
                "accepted GitHub run rows belong under `## Accepted historical evidence`"
                    .to_owned(),
            ));
        }
        if !lower.contains("commit")
            || !lower.contains("tree")
            || hexadecimal_identifiers(trimmed) < 2
        {
            report.push(HygieneViolation::new(
                Some(path.to_path_buf()),
                Some(index + 1),
                RULE_DOCUMENTATION_AUTHORITY,
                "an accepted historical GitHub run row must name its exact full commit and tree"
                    .to_owned(),
            ));
        }
    }
}

fn scan_duplicate_evidence_authority(path: &Path, content: &str, report: &mut HygieneReport) {
    for (index, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        let copied_heading = matches!(
            lower.as_str(),
            "## support matrix" | "## accepted evidence" | "## accepted historical evidence"
        );
        if copied_heading || has_actions_run_link(&lower) {
            report.push(HygieneViolation::new(
                Some(path.to_path_buf()),
                Some(index + 1),
                RULE_DOCUMENTATION_AUTHORITY,
                "current component, overview, and execution guides must not duplicate the support matrix or accepted-run ledger owned by implementation status"
                    .to_owned(),
            ));
        }
    }
}

fn scan_plan_rows(path: &Path, content: &str, report: &mut HygieneReport) {
    for (index, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|') || !trimmed.ends_with('|') {
            continue;
        }
        let lower = trimmed.to_ascii_lowercase();
        if lower.contains("acceptance")
            && contains_any(
                &lower,
                &["remote", "hosted", "cuda", "quality", "ci", "push"],
            )
        {
            report.push(HygieneViolation::new(
                Some(path.to_path_buf()),
                Some(index + 1),
                RULE_DOCUMENTATION_AUTHORITY,
                "execution-plan tables own source ordering only; exact-commit remote acceptance is an external condition, not a tracked package row"
                    .to_owned(),
            ));
        }
    }
}

fn is_current_reference(path: &Path) -> bool {
    if path == Path::new(IMPLEMENTATION_STATUS)
        || path == Path::new(VALIDATION)
        || path == Path::new(PERFORMANCE)
        || path == Path::new(HISTORY)
    {
        return false;
    }

    path == Path::new("README.md")
        || path == Path::new("docs/README.md")
        || path == Path::new("docs/agent/README.md")
        || path.starts_with(Path::new("docs/project"))
        || path == Path::new("docs/agent/execution/README.md")
        || path == Path::new("docs/agent/execution/current.md")
        || path == Path::new("docs/agent/execution/execution-plan.md")
}

fn markdown_blocks(content: &str) -> Vec<(usize, String)> {
    let mut blocks = Vec::new();
    let mut block = String::new();
    let mut start = 1;

    for (index, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            if !block.is_empty() {
                blocks.push((start, std::mem::take(&mut block)));
            }
            continue;
        }
        if block.is_empty() {
            start = index + 1;
        } else {
            block.push(' ');
        }
        block.push_str(line.trim());
    }
    if !block.is_empty() {
        blocks.push((start, block));
    }
    blocks
}

fn contains_current_checkout_subject(value: &str) -> bool {
    contains_any(
        value,
        &[
            "current tree",
            "current checkout",
            "current head",
            "current-commit",
            "this tree",
            "this commit",
            "this candidate",
            "the candidate",
            "source candidate",
            "source-closure candidate",
            "source/test candidate",
        ],
    )
}

fn contains_remote_evidence_subject(value: &str) -> bool {
    contains_any(
        value,
        &["remote", "hosted", "cuda", "quality", "push", "acceptance"],
    ) || contains_ascii_word(value, "ci")
        || contains_ascii_word(value, "run")
        || contains_ascii_word(value, "runs")
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn has_actions_run_link(value: &str) -> bool {
    value.contains("github.com/") && value.contains("/actions/runs/")
}

fn contains_run_reference(value: &str) -> bool {
    has_actions_run_link(value)
        || contains_ascii_word(value, "run")
        || contains_ascii_word(value, "runs")
}

fn contains_ascii_word(value: &str, word: &str) -> bool {
    value.match_indices(word).any(|(position, _)| {
        let before = value[..position].chars().next_back();
        let after = value[position + word.len()..].chars().next();
        before.is_none_or(|character| !is_word_character(character))
            && after.is_none_or(|character| !is_word_character(character))
    })
}

const fn is_word_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || character == '_'
}

fn hexadecimal_identifiers(value: &str) -> usize {
    value
        .split(|character: char| !character.is_ascii_hexdigit())
        .filter(|token| token.len() == 40)
        .count()
}
