use std::env;
use std::path::Path;

use cargo_metadata::{Metadata, MetadataCommand};

pub(crate) fn load_metadata(
    manifest_path: &Path,
    no_deps: bool,
) -> Result<Metadata, cargo_metadata::Error> {
    let mut command = MetadataCommand::new();
    command
        .manifest_path(manifest_path)
        .other_options(vec!["--locked".to_owned()]);
    if no_deps {
        command.no_deps();
    }
    if let Some(cargo) = env::var_os("CARGO") {
        command.cargo_path(cargo);
    }
    command.exec()
}
