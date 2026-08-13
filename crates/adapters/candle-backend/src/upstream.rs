//! Explicit review gate for the Candle implementation modeled by this adapter.

const REVIEWED_VERSION: &str = "0.11.0";
const REVIEWED_PACKAGES: [(&str, &str); 3] = [
    (
        "candle-core",
        "5ecb245093b0f791b89d3420c3df9c6d49c60ab63ba54db896bf8a3baf486706",
    ),
    (
        "candle-nn",
        "eaa10b6ccc365b33210ce404fbf45e60d3e0bdac1004463cf1052e6ee1c1739a",
    ),
    (
        "candle-transformers",
        "3bcbbf7ff00ff6fe2af22b93600195917fe90e90ff48424a140d1a926c44b1c1",
    ),
];

#[test]
fn candle_packages_and_registry_artifacts_require_explicit_review() {
    let manifest = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Cargo.toml"));
    let lock = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Cargo.lock"));

    for (package, checksum) in REVIEWED_PACKAGES {
        let dependency = format!("{package} = {{ version = \"={REVIEWED_VERSION}\"");
        assert!(
            manifest.contains(&dependency),
            "{package} changed: review Candle behavior and sequence formulas before updating the lock"
        );

        let package_marker = format!("name = \"{package}\"\nversion = \"{REVIEWED_VERSION}\"");
        let package_block = lock
            .find(&package_marker)
            .and_then(|package_start| lock.get(package_start..))
            .and_then(|remaining| remaining.split("\n\n").next())
            .unwrap_or_default();
        assert!(
            !package_block.is_empty(),
            "{package} {REVIEWED_VERSION} is absent from Cargo.lock; review required"
        );
        assert!(
            package_block.contains(&format!("checksum = \"{checksum}\"")),
            "{package} registry artifact changed: source review required"
        );
    }
}
