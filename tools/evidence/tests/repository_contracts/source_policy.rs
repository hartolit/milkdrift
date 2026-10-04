//! Structural checks do not interpret comments or literals as Rust declarations.

use syn::{
    Attribute, Meta, Token, UseTree, Visibility,
    parse::Parser,
    punctuated::Punctuated,
    visit::{self, Visit},
};

use super::{TestResult, collect_files, read, root};

#[derive(Default)]
struct Structure {
    exports: Vec<(String, Vec<Attribute>)>,
    public_items: Vec<String>,
    private_field_violations: Vec<String>,
    attributes: Vec<Attribute>,
    calls: Vec<String>,
    macros: Vec<String>,
    constants: Vec<String>,
    types: Vec<String>,
    functions: Vec<String>,
    errors: Vec<String>,
    broad_suppressions: Vec<Attribute>,
}

fn use_paths(tree: &UseTree, prefix: &str, paths: &mut Vec<String>) {
    match tree {
        UseTree::Path(path) => use_paths(&path.tree, &format!("{prefix}{}::", path.ident), paths),
        UseTree::Name(name) => paths.push(format!("{prefix}{}", name.ident)),
        UseTree::Rename(rename) => paths.push(format!("{prefix}{}", rename.ident)),
        UseTree::Glob(_) => paths.push(format!("{prefix}*")),
        UseTree::Group(group) => {
            for item in &group.items {
                use_paths(item, prefix, paths);
            }
        }
    }
}

fn path_name(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

impl<'ast> Visit<'ast> for Structure {
    fn visit_file(&mut self, file: &'ast syn::File) {
        self.broad_suppressions.extend(file.attrs.iter().cloned());
        visit::visit_file(self, file);
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        self.broad_suppressions.extend(item.attrs.iter().cloned());
        visit::visit_item_mod(self, item);
    }
    fn visit_item_enum(&mut self, item: &'ast syn::ItemEnum) {
        self.types.push(item.ident.to_string());
        visit::visit_item_enum(self, item);
    }

    fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
        self.types.push(item.ident.to_string());
        visit::visit_item_type(self, item);
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        self.functions.push(item.sig.ident.to_string());
        visit::visit_trait_item_fn(self, item);
    }
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        self.attributes.push(attribute.clone());
        visit::visit_attribute(self, attribute);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        let mut imports = Vec::new();
        use_paths(&item.tree, "", &mut imports);
        for path in &imports {
            if ["std::include", "core::include"].contains(&path.as_str()) {
                self.errors
                    .push("textual include import bypasses named modules".to_owned());
            }
        }
        if !matches!(item.vis, Visibility::Inherited) {
            let mut paths = Vec::new();
            use_paths(&item.tree, "", &mut paths);
            for path in paths {
                if path.ends_with('*') {
                    self.errors.push(format!("wildcard export {path}"));
                }
                self.exports.push((path, item.attrs.clone()));
            }
        }
        visit::visit_item_use(self, item);
    }

    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        let name = item.ident.to_string();
        self.types.push(name.clone());
        if ["Selection", "BlueprintRevision", "BoundedJson"].contains(&name.as_str()) {
            for field in &item.fields {
                if matches!(field.vis, Visibility::Public(_)) {
                    self.private_field_violations
                        .push(format!("{name} has an externally writable field"));
                }
            }
        }
        visit::visit_item_struct(self, item);
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.functions.push(item.sig.ident.to_string());
        if matches!(item.vis, Visibility::Public(_)) {
            self.public_items.push(item.sig.ident.to_string());
        }
        visit::visit_impl_item_fn(self, item);
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.functions.push(item.sig.ident.to_string());
        visit::visit_item_fn(self, item);
    }

    fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
        self.constants.push(item.ident.to_string());
        if matches!(item.vis, Visibility::Public(_)) {
            self.public_items.push(item.ident.to_string());
        }
        visit::visit_item_const(self, item);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.calls.push(call.method.to_string());
        // This exact established lexical rule has one canonical owner. It is intentionally
        // narrower than claiming to resolve arbitrary equivalent digest implementations.
        if call.method == "strip_prefix" && call.args.first().is_some_and(|arg|
            matches!(arg, syn::Expr::Lit(lit) if matches!(&lit.lit, syn::Lit::Str(text) if text.value() == "b3_"))) {
            self.calls.push("strip_b3_prefix".to_owned());
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_macro(&mut self, item: &'ast syn::Macro) {
        if ["include", "std::include", "core::include"].contains(&path_name(&item.path).as_str()) {
            self.errors
                .push("textual include bypasses named modules".to_owned());
        }
        self.macros.push(path_name(&item.path));
        // Macro bodies are opaque to syn's AST. Inspect attribute token groups as well so an
        // allow hidden in macro_rules cannot evade suppression policy. Expansion/type meaning
        // remains compiler evidence, not a promise of this source visitor.
        macro_attributes(item.tokens.clone(), &mut self.attributes);
        visit::visit_macro(self, item);
    }
}

fn macro_attributes(tokens: proc_macro2::TokenStream, attributes: &mut Vec<Attribute>) {
    let mut previous_hash = false;
    for token in tokens {
        match token {
            proc_macro2::TokenTree::Group(group) => {
                if previous_hash && group.delimiter() == proc_macro2::Delimiter::Bracket {
                    let source = format!("#[{}]", group.stream());
                    if let Ok(found) = Attribute::parse_outer.parse_str(&source) {
                        attributes.extend(found);
                    }
                }
                macro_attributes(group.stream(), attributes);
                previous_hash = false;
            }
            proc_macro2::TokenTree::Punct(punct) => {
                if punct.as_char() != '!' {
                    previous_hash = punct.as_char() == '#';
                }
            }
            _ => previous_hash = false,
        }
    }
}

fn structure(source: &str) -> TestResult<Structure> {
    let syntax = syn::parse_file(source)?;
    let mut result = Structure::default();
    result.visit_file(&syntax);
    Ok(result)
}

fn private_constant(owner: &str, name: &str) -> bool {
    match owner {
        "authority" => name == "AUTHORITY_GRANT_SCHEMA_VERSION",
        "blueprint" => name == "BLUEPRINT_SCHEMA_VERSION",
        "capability" => name.contains("SCHEMA_VERSION") || name == "MAX_JSON_DEPTH",
        "control-protocol" => name.starts_with("PROTOCOL_") || name == "LAYOUT_SCHEMA_VERSION",
        "model" => [
            "MODEL_CONTRACT_SCHEMA_VERSION",
            "CONTEXT_MANIFEST_SCHEMA_VERSION",
        ]
        .contains(&name),
        "prompt-sequence" => name == "PROMPT_SEQUENCE_SCHEMA_VERSION",
        "local-process" => name == "PROCESS_PROFILE_SCHEMA_VERSION",
        "model-provider" => name == "MODEL_ENDPOINT_PROFILE_SCHEMA_VERSION",
        _ => false,
    }
}

fn suppression_errors(meta: &Meta, errors: &mut Vec<String>) -> syn::Result<()> {
    let Meta::List(list) = meta else {
        return Ok(());
    };
    if list.path.is_ident("cfg_attr") {
        let children = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for child in children.iter().skip(1) {
            suppression_errors(child, errors)?;
        }
    } else if list.path.is_ident("allow") || list.path.is_ident("expect") {
        let values = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        let reason = values.iter().find_map(|value| match value {
            Meta::NameValue(pair) if pair.path.is_ident("reason") => match &pair.value {
                syn::Expr::Lit(lit) => match &lit.lit {
                    syn::Lit::Str(text) => Some(text.value()),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        });
        if reason
            .as_deref()
            .is_none_or(|text| text.split_whitespace().count() < 4)
        {
            errors.push("suppression needs a specific reason attribute".to_owned());
        }
        for value in values {
            if let Meta::Path(path) = value {
                let name = path_name(&path);
                if [
                    "warnings",
                    "unused",
                    "rust_2018_idioms",
                    "clippy::all",
                    "clippy::pedantic",
                    "clippy::nursery",
                    "clippy::restriction",
                    "unsafe_code",
                    "unused_must_use",
                    "unknown_lints",
                    "renamed_and_removed_lints",
                    "unfulfilled_lint_expectations",
                    "unexpected_cfgs",
                ]
                .contains(&name.as_str())
                {
                    errors.push(format!("broad or integrity suppression: {name}"));
                }
            }
        }
    }
    Ok(())
}

fn contains_suppression(meta: &Meta) -> syn::Result<bool> {
    if meta.path().is_ident("allow") || meta.path().is_ident("expect") {
        return Ok(true);
    }
    if let Meta::List(list) = meta
        && list.path.is_ident("cfg_attr")
    {
        for child in list
            .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?
            .iter()
            .skip(1)
        {
            if contains_suppression(child)? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn cfg_enabled(meta: &Meta, helpers: bool) -> syn::Result<bool> {
    match meta {
        Meta::Path(path) => Ok(!path.is_ident("test")),
        Meta::NameValue(value) if value.path.is_ident("feature") => Ok(helpers),
        Meta::List(list) => {
            let values = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
            let flags = values
                .iter()
                .map(|v| cfg_enabled(v, helpers))
                .collect::<syn::Result<Vec<_>>>()?;
            if list.path.is_ident("any") {
                Ok(flags.iter().any(|v| *v))
            } else if list.path.is_ident("all") || list.path.is_ident("cfg") {
                Ok(flags.iter().all(|v| *v))
            } else if list.path.is_ident("not") && flags.len() == 1 {
                Ok(!flags.into_iter().all(|v| v))
            } else {
                Err(syn::Error::new_spanned(
                    meta,
                    "unsupported export cfg; require a compiler probe",
                ))
            }
        }
        Meta::NameValue(_) => Err(syn::Error::new_spanned(meta, "unsupported export cfg")),
    }
}

fn export_enabled(meta: &Meta, helpers: bool) -> syn::Result<bool> {
    if meta.path().is_ident("cfg") {
        return cfg_enabled(meta, helpers);
    }
    if let Meta::List(list) = meta
        && list.path.is_ident("cfg_attr")
    {
        let values = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        let condition = values
            .first()
            .ok_or_else(|| syn::Error::new_spanned(meta, "empty conditional attribute"))?;
        if cfg_enabled(condition, helpers)? {
            for child in values.iter().skip(1) {
                if !export_enabled(child, helpers)? {
                    return Ok(false);
                }
            }
        }
    }
    Ok(true)
}

#[test]
fn syntax_checks_explicit_exports_private_values_and_canonical_owners() -> TestResult {
    let repository = root()?;
    let mut sources = Vec::new();
    collect_files(&repository, &mut sources, &|path| {
        path.extension().is_some_and(|e| e == "rs")
    })?;
    for path in sources {
        let syntax = structure(&read(&path)?)?;
        let relative = path
            .strip_prefix(&repository)?
            .to_string_lossy()
            .replace('\\', "/");
        assert!(syntax.errors.is_empty(), "{relative}: {:?}", syntax.errors);
        if [
            "crates/authority/src/selection.rs",
            "crates/blueprint/src/revision.rs",
            "crates/capability/src/bounded.rs",
        ]
        .contains(&relative.as_str())
        {
            assert!(
                syntax.private_field_violations.is_empty(),
                "{relative}: {:?}",
                syntax.private_field_violations
            );
        }
        if relative == "crates/blueprint/src/revision.rs" {
            assert!(
                !syntax
                    .public_items
                    .iter()
                    .any(|name| name == "from_verified_parts" || name == "publish"),
                "raw revision construction became external"
            );
        }
        if relative.starts_with("crates/contracts/src/") {
            assert!(
                !syntax
                    .constants
                    .iter()
                    .any(|name| name.contains("SCHEMA_VERSION")),
                "shared mechanics own domain schema in {relative}"
            );
        }
        if ["crates/", "adapters/", "apps/"]
            .iter()
            .any(|prefix| relative.starts_with(prefix))
            && relative != "crates/contracts/src/text.rs"
        {
            assert!(
                !syntax
                    .calls
                    .iter()
                    .any(|name| name == "is_char_boundary" || name == "strip_b3_prefix"),
                "shared lexical mechanics duplicated in {relative}"
            );
        }
        if relative.ends_with("/src/lib.rs") {
            for (export, attributes) in &syntax.exports {
                let forbidden = match relative.as_str() {
                    "crates/authority/src/lib.rs" | "crates/peer-protocol/src/lib.rs" => {
                        export == "milkdrift_capability::PeerId"
                    }
                    "crates/workspace/src/lib.rs" => export == "milkdrift_capability::BoundedJson",
                    "crates/model/src/lib.rs" => export.starts_with("milkdrift_capability::"),
                    "crates/persistence/src/lib.rs" => {
                        export == "milkdrift_authority::ActorRef"
                            || export == "milkdrift_capability::InvocationId"
                            || export.starts_with("milkdrift_workspace::")
                    }
                    "apps/daemon/src/lib.rs" => export == "http::router",
                    _ => false,
                };
                assert!(!forbidden, "alternate owner export in {relative}: {export}");
                if [
                    "ManualClock",
                    "DeterministicExecutor",
                    "InMemorySecretResolver",
                    "SystemPeerClock",
                    "execute_exact",
                    "FaultInjector",
                    "FaultPoint",
                    "injected_failure",
                ]
                .iter()
                .any(|name| export.rsplit("::").next() == Some(name))
                {
                    let conditions = attributes
                        .iter()
                        .map(|attr| export_enabled(&attr.meta, false))
                        .collect::<syn::Result<Vec<_>>>()?;
                    assert!(
                        conditions.iter().any(|enabled| !enabled),
                        "helper exported by default in {relative}: {export}"
                    );
                }
                let owner = path
                    .parent()
                    .and_then(|p| p.parent())
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .ok_or("missing package owner")?;
                assert!(
                    !private_constant(owner, export.rsplit("::").next().ok_or("empty export")?),
                    "private implementation constant exported in {relative}: {export}"
                );
            }
            let owner = path
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .ok_or("missing package owner")?;
            assert!(
                !syntax
                    .public_items
                    .iter()
                    .any(|name| private_constant(owner, name)),
                "private constant exported in {relative}"
            );
        }
    }
    for (path, removed) in [
        (
            "crates/runtime/src/engine.rs",
            &["effect_tick", "drive_once"][..],
        ),
        (
            "crates/runtime/src/engine/effects.rs",
            &["effect_tick", "drive_once"][..],
        ),
        ("crates/runtime/src/engine/scheduling.rs", &["tick"][..]),
        (
            "crates/persistence/src/journal/query.rs",
            &["nonterminal_runs", "runnable"][..],
        ),
        ("crates/runtime/src/executor.rs", &["execute_streaming"][..]),
    ] {
        let found = structure(&read(repository.join(path))?)?;
        assert!(
            !found
                .functions
                .iter()
                .any(|name| removed.contains(&name.as_str())),
            "removed compatibility entry in {path}"
        );
        assert!(
            !found
                .types
                .iter()
                .any(|name| ["EffectTickResult", "ExecutionReportBatch"].contains(&name.as_str())),
            "removed compatibility type in {path}"
        );
    }
    assert!(
        structure(&read(repository.join("crates/runtime/src/executor.rs"))?)?
            .functions
            .iter()
            .any(|name| name == "prepare_exact_entry")
    );
    for (path, name) in [
        ("crates/capability/src/bounded.rs", "BoundedJson"),
        ("crates/blueprint/src/model/contract.rs", "SchemaRef"),
        ("crates/capability/src/descriptor.rs", "SchemaContract"),
    ] {
        assert!(
            structure(&read(repository.join(path))?)?
                .types
                .iter()
                .any(|found| found == name)
        );
    }
    for consumer in [
        "crates/blueprint/src/model/contract.rs",
        "crates/capability/src/descriptor.rs",
    ] {
        assert!(
            structure(&read(repository.join(consumer))?)?
                .macros
                .iter()
                .any(|name| name == "milkdrift_contracts::deserialize_via")
        );
    }
    let schema = "RESOLVED_CAPABILITY_SNAPSHOT_SCHEMA_VERSION_V3";
    assert_eq!(
        structure(&read(repository.join("crates/capability/src/document.rs"))?)?
            .constants
            .iter()
            .filter(|name| *name == schema)
            .count(),
        1
    );
    assert!(
        !structure(&read(repository.join("crates/capability/src/resolved.rs"))?)?
            .constants
            .iter()
            .any(|name| name == schema)
    );
    Ok(())
}

pub(super) fn repository_suppression_errors() -> TestResult<Vec<String>> {
    let repository = root()?;
    let mut sources = Vec::new();
    collect_files(&repository, &mut sources, &|path| {
        path.extension().is_some_and(|e| e == "rs")
    })?;
    let mut errors = Vec::new();
    for path in sources {
        let syntax = structure(&read(&path)?)?;
        for attribute in syntax.broad_suppressions {
            if contains_suppression(&attribute.meta)? {
                errors.push(format!(
                    "{}:{}: file/module suppression must be narrowed to the actual operation",
                    path.display(),
                    attribute.pound_token.span.start().line
                ));
            }
        }
        for attribute in syntax.attributes {
            let mut found = Vec::new();
            suppression_errors(&attribute.meta, &mut found)?;
            errors.extend(found.into_iter().map(|error| {
                format!(
                    "{}:{}: {error}",
                    path.display(),
                    attribute.pound_token.span.start().line
                )
            }));
        }
    }
    Ok(errors)
}

#[test]
fn suppressions_are_narrow_and_carry_reasons() -> TestResult {
    let errors = repository_suppression_errors()?;
    assert!(errors.is_empty(), "{}", errors.join("\n"));
    Ok(())
}

#[test]
fn syntax_accepts_formatting_literals_grouped_exports_and_rejects_bypasses() -> TestResult {
    let good = structure(
        r#"
        // pub use evil::*;
        const TEXT: &str = "pub use wrong::*; #[allow(warnings)]";
        pub(crate) use owner::{Name, nested::{Other as Alias}};
        macro_rules! valid { () => { pub struct Generated; }; }
        struct Selection { private: bool }
    "#,
    )?;
    assert!(good.errors.is_empty() && good.private_field_violations.is_empty());
    let broad = structure(
        "#[allow(dead_code, reason=\"An entire module cannot receive a blanket exception\")] mod hidden {}",
    )?;
    assert!(
        broad
            .broad_suppressions
            .iter()
            .any(|attr| matches!(contains_suppression(&attr.meta), Ok(true)))
    );
    assert!(
        good.exports
            .iter()
            .any(|(name, _)| name == "owner::nested::Other")
    );
    let bad = structure(
        "pub(in crate::owner) use source::{A, nested::*}; struct Selection { pub raw: bool }",
    )?;
    assert_eq!(bad.errors.len(), 1);
    assert_eq!(bad.private_field_violations.len(), 1);
    assert!(structure("fn invalid(").is_err());
    let conditional = structure(
        "#[cfg_attr(not(test), cfg(feature=\"test-support\"))] pub use owner::ManualClock;",
    )?;
    for (_, attributes) in conditional.exports {
        let defaults = attributes
            .iter()
            .map(|attribute| export_enabled(&attribute.meta, false))
            .collect::<syn::Result<Vec<_>>>()?;
        let helpers = attributes
            .iter()
            .map(|attribute| export_enabled(&attribute.meta, true))
            .collect::<syn::Result<Vec<_>>>()?;
        assert!(defaults.contains(&false));
        assert!(helpers.iter().all(|enabled| *enabled));
    }
    for source in [
        "include!(\"hidden.rs\");",
        "use std::{include as hidden}; hidden!(\"hidden.rs\");",
    ] {
        assert!(!structure(source)?.errors.is_empty());
    }
    let good = structure(
        "#[expect(dead_code, reason = \"Fixture tests an intentionally unused boundary\")] fn unused() {}",
    )?;
    let mut errors = Vec::new();
    for attr in good.attributes {
        suppression_errors(&attr.meta, &mut errors)?;
    }
    assert!(errors.is_empty());
    for source in [
        "#[cfg_attr(test, cfg_attr(unix, allow(clippy::all, reason = \"Hide every diagnostic in this fixture\")))] fn bad() {}",
        "#[expect(dead_code)] fn bad() {}",
        "macro_rules! hidden { () => { #[allow(warnings)] fn bad() {} }; }",
    ] {
        let mut errors = Vec::new();
        for attr in structure(source)?.attributes {
            suppression_errors(&attr.meta, &mut errors)?;
        }
        assert!(!errors.is_empty(), "missed {source}");
    }
    // Exercise the repository traversal before activation without accepting its existing findings.
    let _existing_findings = repository_suppression_errors()?;
    Ok(())
}
