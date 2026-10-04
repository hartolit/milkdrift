//! Repository-level documentation, dependency-direction, and public-boundary contracts.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs,
    path::{Path, PathBuf},
};

use milkdrift_local_process::ProcessProfileDocument;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const COHESION_REVIEW_LINES: usize = 1_000;
const MAXIMUM_SOURCE_LINES: usize = 1_500;

#[derive(Clone, Copy, Debug)]
struct CohesionException {
    path: &'static str,
    ceiling: usize,
    rationale: &'static str,
}

const PRODUCTION_COHESION_EXCEPTIONS: &[CohesionException] = &[
    CohesionException {
        path: "adapters/local-process/src/config.rs",
        ceiling: 1_006,
        rationale: "profile decoding checks executable identity, substitution/input references, filesystem access, stream secrecy, and aggregate output bounds before registration",
    },
    CohesionException {
        path: "adapters/model-provider/src/adapter.rs",
        ceiling: 1_155,
        rationale: "exact endpoint entry binds selected context materialization to bounded HTTP observations and refuses successful publication after incomplete provider responses",
    },
    CohesionException {
        path: "crates/control/src/service.rs",
        ceiling: 1_290,
        rationale: "proposal submission and approval preserve candidate validation, actor authority, controller assessment, revision storage, and prospective runtime acceptance ordering",
    },
];

fn root() -> TestResult<PathBuf> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| {
            std::io::Error::other("evidence package must remain under tools/evidence")
        })?;
    Ok(path.to_path_buf())
}

fn read(path: impl AsRef<Path>) -> TestResult<String> {
    Ok(fs::read_to_string(path)?)
}

fn numeric_const(relative: &str, name: &str) -> TestResult<u64> {
    let source = read(root()?.join(relative))?;
    let marker = format!("const {name}:");
    let line = source
        .lines()
        .find(|line| line.contains(&marker))
        .ok_or_else(|| std::io::Error::other(format!("{relative} has no {name}")))?;
    let literal = line
        .split_once('=')
        .ok_or_else(|| std::io::Error::other("constant must have an initializer"))?
        .1
        .trim()
        .trim_end_matches(';');
    Ok(literal.parse()?)
}

#[test]
fn repository_evidence_tasks_have_one_cargo_owned_rust_path() -> TestResult {
    let repository = root()?;
    let mut interpreter_sources = Vec::new();
    collect_files(&repository, &mut interpreter_sources, &|path| {
        path.extension().is_some_and(|extension| extension == "py")
    })?;
    assert!(
        interpreter_sources.is_empty(),
        "maintained tooling and fixtures must use Rust: {interpreter_sources:?}"
    );
    let aliases = read(repository.join(".cargo/config.toml"))?;
    assert!(aliases.contains(
        "external-evidence = \"run --package milkdrift-evidence --bin milkdrift-external-evidence --\""
    ));
    assert!(aliases.contains(
        "mutation-evidence = \"run --package milkdrift-evidence --bin mutation-evidence --\""
    ));
    assert!(
        repository
            .join("tools/evidence/src/bin/mutation-evidence.rs")
            .is_file()
    );
    assert!(
        repository
            .join("tools/evidence/src/bin/milkdrift-external-evidence/main.rs")
            .is_file()
    );
    for obsolete in [
        "scripts/check-mutation-classifications.mjs",
        "scripts/run-mutation-shard.sh",
        "scripts/run-external-evidence.sh",
        "apps/daemon/src/bin/milkdrift-external-evidence/main.rs",
    ] {
        assert!(
            !repository.join(obsolete).is_file(),
            "obsolete repository task path returned: {obsolete}"
        );
    }

    for path in [
        "tools/evidence/src/daemon.rs",
        "tools/evidence/src/bin/milkdrift-external-evidence/main.rs",
    ] {
        let source = read(repository.join(path))?;
        assert!(
            !source.contains("DaemonHost::start"),
            "application evidence bypassed the product binary: {path}"
        );
        assert!(
            !source.contains("DaemonPlan"),
            "application evidence owns compiled daemon state: {path}"
        );
    }
    let workflow = read(repository.join(".github/workflows/mutation.yml"))?;
    assert!(workflow.contains(
        "cargo mutation-evidence \"$MUTATION_SHARD\" --partition \"$MUTATION_PARTITION\""
    ));
    for document in [
        "README.md",
        "docs/development/workflow.md",
        "docs/guides/external-evidence.md",
        "docs/development/verification-evidence.md",
    ] {
        let contents = read(repository.join(document))?;
        assert!(
            !contents.contains("scripts/"),
            "obsolete script path remains documented in {document}"
        );
    }
    Ok(())
}

#[test]
fn rust_modules_use_named_file_children_and_respect_the_size_backstop() -> TestResult {
    let repository = root()?;
    let mut sources = Vec::new();
    collect_files(&repository, &mut sources, &|path| {
        path.extension().and_then(|extension| extension.to_str()) == Some("rs")
    })?;
    for source in sources {
        assert_ne!(
            source.file_name().and_then(|name| name.to_str()),
            Some("mod.rs"),
            "module ownership must use file.rs with file/ children: {}",
            source.display()
        );
        let size = source_size::source_size(&read(&source)?)
            .map_err(|error| format!("{}: {error}", source.display()))?;
        assert!(
            size.implementation < MAXIMUM_SOURCE_LINES,
            "{} has {} implementation lines ({} physical); perform the required cohesion review before crossing the {MAXIMUM_SOURCE_LINES}-line backstop",
            source.display(),
            size.implementation,
            size.physical
        );
    }
    Ok(())
}

#[test]
fn production_sources_over_the_review_threshold_have_exact_bounded_exceptions() -> TestResult {
    let repository = root()?;
    let mut paths = Vec::new();
    collect_files(&repository, &mut paths, &|path| {
        path.extension().and_then(|extension| extension.to_str()) == Some("rs")
    })?;
    let sources = paths
        .iter()
        .map(|path| {
            let relative = path
                .strip_prefix(&repository)
                .map_err(|_| {
                    std::io::Error::other(format!(
                        "collected source escaped repository root: {}",
                        path.display()
                    ))
                })?
                .to_string_lossy()
                .replace('\\', "/");
            let size = source_size::source_size(&read(path)?)
                .map_err(|error| format!("{relative}: {error}"))?;
            Ok(SourceLineCount {
                production: is_production_source(&relative),
                path: relative,
                lines: size.implementation,
                physical_lines: size.physical,
            })
        })
        .collect::<TestResult<Vec<_>>>()?;
    for source in sources
        .iter()
        .filter(|source| source.production && source.lines > COHESION_REVIEW_LINES)
    {
        println!(
            "cohesion review: {} has {} implementation lines ({} physical)",
            source.path, source.lines, source.physical_lines
        );
    }
    let errors = cohesion_policy_errors(PRODUCTION_COHESION_EXCEPTIONS, &sources);
    assert!(
        errors.is_empty(),
        "production cohesion exception policy failed:\n{}",
        errors.join("\n")
    );
    Ok(())
}

#[test]
fn cohesion_policy_rejects_missing_stale_duplicate_over_broad_and_exceeded_exceptions() {
    let sources = vec![
        SourceLineCount::production("crates/example/src/missing.rs", 1_001),
        SourceLineCount::production("crates/example/src/stale.rs", 900),
        SourceLineCount::production("crates/example/src/duplicate.rs", 1_001),
        SourceLineCount::production("crates/example/src/exceeded.rs", 1_101),
        SourceLineCount::production("crates/example/src/generic.rs", 1_001),
        SourceLineCount {
            path: "crates/example/tests/large.rs".to_owned(),
            lines: 1_500,
            physical_lines: 1_500,
            production: false,
        },
    ];
    let exceptions = [
        CohesionException {
            path: "crates/example/src/stale.rs",
            ceiling: 1_050,
            rationale: "this deliberately stale fixture has enough words for policy validation",
        },
        CohesionException {
            path: "crates/example/src/duplicate.rs",
            ceiling: 1_050,
            rationale: "this deliberately duplicate fixture has enough words for policy validation",
        },
        CohesionException {
            path: "crates/example/src/duplicate.rs",
            ceiling: 1_060,
            rationale: "this second duplicate fixture has enough words for policy validation",
        },
        CohesionException {
            path: "crates/example/src/*",
            ceiling: 1_050,
            rationale: "this deliberately broad fixture has enough words for policy validation",
        },
        CohesionException {
            path: "crates/example/src/exceeded.rs",
            ceiling: 1_050,
            rationale: "this deliberately exceeded fixture has enough words for policy validation",
        },
        CohesionException {
            path: "crates/example/src/generic.rs",
            ceiling: 1_050,
            rationale: "one owner implements the complete contract in this file",
        },
    ];
    let errors = cohesion_policy_errors(&exceptions, &sources).join("\n");
    for category in [
        "missing",
        "stale",
        "duplicate",
        "over-broad",
        "exceeded",
        "weak rationale",
    ] {
        assert!(
            errors.contains(category),
            "missing {category} diagnostic: {errors}"
        );
    }
    assert!(
        !errors.contains("crates/example/tests/large.rs"),
        "test and evidence sources must remain separate from production review exceptions"
    );
}

#[test]
fn exact_current_process_fixture_respects_the_host_reader() -> TestResult {
    let fixtures = root()?.join("adapters/local-process/tests/fixtures");
    let current = fs::read(fixtures.join("process-profile-v2.json"))?;
    if cfg!(unix) {
        let document = ProcessProfileDocument::from_json(&current)?;
        let canonical = document.to_canonical_json()?;
        assert_eq!(
            ProcessProfileDocument::from_json(&canonical)?.to_canonical_json()?,
            canonical
        );
    } else {
        // Profiles bind host paths and platform ownership, unlike portable blueprint documents.
        // The Unix golden must be refused by a Windows reader, not silently reinterpreted.
        let error = ProcessProfileDocument::from_json(&current)
            .err()
            .ok_or("foreign host profile was accepted")?;
        assert!(
            error
                .to_string()
                .contains("executable must be an absolute NUL-free path")
        );
    }
    let legacy = fs::read(fixtures.join("process-profile-v1.json"))?;
    assert!(ProcessProfileDocument::from_json(&legacy).is_err());
    Ok(())
}

fn collect_files(
    directory: &Path,
    files: &mut Vec<PathBuf>,
    selects: &impl Fn(&Path) -> bool,
) -> std::io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            if !matches!(
                path.file_name().and_then(|name| name.to_str()),
                Some("target" | ".git")
            ) {
                collect_files(&path, files, selects)?;
            }
        } else if selects(&path) {
            files.push(path);
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct SourceLineCount {
    path: String,
    lines: usize,
    physical_lines: usize,
    production: bool,
}

impl SourceLineCount {
    fn production(path: &str, lines: usize) -> Self {
        Self {
            path: path.to_owned(),
            lines,
            physical_lines: lines,
            production: true,
        }
    }
}

fn is_production_source(relative: &str) -> bool {
    let mut components = relative.split('/');
    let Some(area) = components.next() else {
        return false;
    };
    if !matches!(area, "adapters" | "apps" | "crates") || !relative.contains("/src/") {
        return false;
    }
    !relative
        .split('/')
        .any(|component| matches!(component, "tests" | "tests.rs" | "testing.rs"))
}

fn cohesion_policy_errors(
    exceptions: &[CohesionException],
    sources: &[SourceLineCount],
) -> Vec<String> {
    let source_by_path = sources
        .iter()
        .map(|source| (source.path.as_str(), source))
        .collect::<BTreeMap<_, _>>();
    let mut exception_by_path = BTreeMap::new();
    let mut errors = Vec::new();
    let mut prior_path = None;
    for exception in exceptions {
        if prior_path.is_some_and(|prior| prior >= exception.path) {
            errors.push(format!(
                "unordered exception: {} must follow exact lexical path order",
                exception.path
            ));
        }
        prior_path = Some(exception.path);
        let exact_path = exception.path.ends_with(".rs")
            && !exception.path.starts_with('/')
            && !exception.path.contains("..")
            && !exception.path.contains(['*', '?', '[', ']'])
            && is_production_source(exception.path);
        if !exact_path {
            errors.push(format!("over-broad exception: {}", exception.path));
            continue;
        }
        if exception.rationale.split_whitespace().count() < 6
            || exception.rationale.contains("complete contract")
            || exception
                .rationale
                .contains("complete external adapter contract")
            || exception.rationale.contains("complete versioned")
        {
            errors.push(format!("empty or weak rationale: {}", exception.path));
        }
        if exception.ceiling <= COHESION_REVIEW_LINES || exception.ceiling >= MAXIMUM_SOURCE_LINES {
            errors.push(format!(
                "invalid bounded ceiling for {}: {}",
                exception.path, exception.ceiling
            ));
        }
        if exception_by_path
            .insert(exception.path, exception)
            .is_some()
        {
            errors.push(format!("duplicate exception: {}", exception.path));
        }
    }

    for exception in exception_by_path.values() {
        match source_by_path.get(exception.path) {
            None => errors.push(format!("stale exception path: {}", exception.path)),
            Some(source) if !source.production || source.lines <= COHESION_REVIEW_LINES => errors
                .push(format!(
                    "stale exception below review threshold: {} has {} implementation lines ({} physical)",
                    exception.path, source.lines, source.physical_lines
                )),
            Some(source) if source.lines > exception.ceiling => errors.push(format!(
                "exceeded exception ceiling: {} has {} implementation lines ({} physical) above {}",
                exception.path, source.lines, source.physical_lines, exception.ceiling
            )),
            Some(_) => {}
        }
    }
    for source in sources
        .iter()
        .filter(|source| source.production && source.lines > COHESION_REVIEW_LINES)
    {
        if !exception_by_path.contains_key(source.path.as_str()) {
            errors.push(format!(
                "missing exception: {} has {} implementation lines ({} physical)",
                source.path, source.lines, source.physical_lines
            ));
        }
    }
    errors
}

#[path = "repository_contracts/documentation.rs"]
mod documentation;

#[path = "repository_contracts/source_size.rs"]
mod source_size;

#[path = "repository_contracts/consumers.rs"]
mod consumers;
#[path = "repository_contracts/dependency_policy.rs"]
mod dependency_policy;
#[path = "repository_contracts/lint_probes.rs"]
mod lint_probes;
#[path = "repository_contracts/manifests.rs"]
mod manifests;
#[path = "repository_contracts/product_graph.rs"]
mod product_graph;
#[path = "repository_contracts/source_policy.rs"]
mod source_policy;
