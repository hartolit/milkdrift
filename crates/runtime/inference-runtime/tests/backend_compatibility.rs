//! Compile-time compatibility checks for concrete backend adapters.

use domain_contracts::ModelLoader;
use gguf_backend::GgufLoader;

#[test]
fn gguf_loader_satisfies_the_runtime_loader_boundary() {
    assert_loader::<GgufLoader>();
}

const fn assert_loader<L: ModelLoader>() {}
