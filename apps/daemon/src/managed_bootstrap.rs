//! Generate one inspectable private host configuration; platform installation remains API-owned.
use milkdrift_authority::{AccessMode, FilesystemScope};
use milkdrift_capability::managed::ManagedName;
use milkdrift_managed_linux::{
    LinuxManagerConfig, LinuxRecipe, ModelService, ProtectedServiceRecipe, WorkerNetwork,
};
use serde_json::json;
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(clap::Args)]
pub(super) struct Arguments {
    /// New private product directory; existing generated files must match exactly.
    #[arg(long)]
    root: PathBuf,
    /// Rootless Quadlet search directory (normally ~/.config/containers/systemd/milkdrift).
    #[arg(long)]
    quadlet_directory: PathBuf,
    /// Rootless systemd unit search directory (normally ~/.config/systemd/user).
    #[arg(long)]
    systemd_directory: PathBuf,
    /// One strict recipe with exact operator-supplied image and optional model inputs.
    #[arg(long)]
    recipe: PathBuf,
    /// Installation name; defaults to the approved recipe name.
    #[arg(long)]
    installation: Option<String>,
    /// Print generated non-secret configuration and recipe identity without writing files.
    #[arg(long)]
    preview: bool,
}

pub(super) fn run(args: Arguments) -> Result<(), Box<dyn std::error::Error>> {
    if !cfg!(target_os = "linux") {
        return Err("managed bootstrap requires Linux".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(&args.recipe)?
        .take(65_537)
        .read_to_end(&mut bytes)?;
    let protected = serde_json::from_slice::<serde_json::Value>(&bytes)?
        .get("kind")
        .and_then(|v| v.as_str())
        == Some("protected_service");
    let recipe = if protected {
        None
    } else {
        Some(LinuxRecipe::from_json(&bytes)?)
    };
    let protected = if protected {
        Some(ProtectedServiceRecipe::from_json(&bytes)?)
    } else {
        None
    };
    let (name, reference, recipe_value) = if let Some(recipe) = &recipe {
        (
            recipe.name.clone(),
            recipe.reference()?,
            serde_json::to_value(recipe)?,
        )
    } else {
        let recipe = protected.as_ref().ok_or("recipe absent")?;
        (
            recipe.name.clone(),
            recipe.reference()?,
            serde_json::to_value(recipe)?,
        )
    };
    let installation = args
        .installation
        .map(ManagedName::new)
        .transpose()?
        .unwrap_or(name);
    let manager = LinuxManagerConfig {
        state_root: args.root.join("manager"),
        quadlet_directory: args.quadlet_directory.clone(),
        systemd_directory: args.systemd_directory.clone(),
        recipes: vec![args.root.join("recipe.json")],
    };
    manager.validate()?;
    let mut config: serde_json::Value = serde_json::to_value(toml::from_str::<toml::Table>(
        include_str!("../../../examples/operator/execution-only.toml"),
    )?)?;
    config["data_root"] = json!(args.root.join("data"));
    config["host_id"] = json!(format!("host:{installation}"));
    config["secret_sources"]["credential:operator"] =
        json!({"type":"file","path":args.root.join("operator.token")});
    config["adapters"]["managed_linux"] = serde_json::to_value(&manager)?;
    let scope = &mut config["actors"][0]["authority"]["resources"];
    scope["capability"]["identities"]["values"] = json!([
        "milkdrift.resources",
        format!("managed.{installation}"),
        format!("managed.{installation}.worker"),
        format!("managed.{installation}.model")
    ]);
    scope["capability"]["operations"]["values"] = json!([
        "resource.manage",
        "resource.evaluate",
        "resource.evidence",
        "resource.publish",
        "resource.evaluate_candidate",
        "resource.publish_candidate",
        "resource.prepare",
        "resource.apply",
        "resource.inspect",
        "resource.start",
        "resource.stop",
        "resource.update",
        "resource.preserve",
        "resource.remove",
        "resource.recover",
        "resource.resolve",
        "resource.handoff",
        "resource.return",
        "workspace.execute",
        "model.generate"
    ]);
    scope["capability"]["trust_zones"] = json!({"type":"any"});
    let mut filesystem = vec![
        FilesystemScope::from_canonical_host_path(
            &manager.state_root,
            BTreeSet::from([AccessMode::Read, AccessMode::Write]),
        )?,
        FilesystemScope::from_canonical_host_path(
            &manager.quadlet_directory,
            BTreeSet::from([AccessMode::Read, AccessMode::Write]),
        )?,
        FilesystemScope::from_canonical_host_path(
            &manager.systemd_directory,
            BTreeSet::from([AccessMode::Read, AccessMode::Write]),
        )?,
    ];
    let mut profiles = Vec::new();
    let mut destinations = Vec::new();
    if recipe
        .as_ref()
        .is_some_and(|r| r.worker_network == WorkerNetwork::Outbound)
    {
        profiles.push("managed.outbound".to_owned());
    }
    let endpoint = match recipe.as_ref().map(|r| &r.model_service) {
        None | Some(ModelService::Disabled {}) => None,
        Some(ModelService::Attached { api_base, .. }) => Some(api_base.clone()),
        Some(ModelService::Owned { model, port, .. }) => {
            filesystem.push(FilesystemScope::from_canonical_host_path(
                model,
                BTreeSet::from([AccessMode::Read]),
            )?);
            Some(format!("http://127.0.0.1:{port}/v1"))
        }
    };
    if let Some(endpoint) = endpoint {
        let endpoint = url::Url::parse(&endpoint)?;
        profiles.push(format!("managed.{installation}.model"));
        profiles.push("managed.service".to_owned());
        destinations.push(format!(
            "{}:{}",
            endpoint.host().ok_or("endpoint host missing")?,
            endpoint
                .port_or_known_default()
                .ok_or("endpoint port missing")?
        ));
    }
    scope["filesystem"] = serde_json::to_value(filesystem)?;
    scope["network"] = json!({"profiles":profiles,"destinations":destinations});
    let config_text = toml::to_string_pretty(&config)?;
    if args.preview {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"recipe":reference,"configuration":config,"required_operator_prerequisites":["rootless Podman 5.4..6.x and preloaded exact images","user systemd session with lingering and cgroup v2 delegation","subordinate UID/GID mappings and private manager directories"],"platform_effects":"none; prepare/apply use the authenticated resource API"})
            )?
        );
        return Ok(());
    }
    private_directory(&args.root)?;
    private_directory(&manager.state_root)?;
    private_directory(&manager.quadlet_directory)?;
    // The shared systemd search root may be readable; each owned drop-in is private.
    if !manager.systemd_directory.exists() {
        private_directory(&manager.systemd_directory)?;
    }
    write_exact(
        &args.root.join("recipe.json"),
        &serde_json::to_vec_pretty(&recipe_value)?,
    )?;
    let token_path = args.root.join("operator.token");
    if !token_path.exists() {
        let mut random = [0; 32];
        fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
        let token = random
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        write_exact(&token_path, token.as_bytes())?;
    }
    write_exact(&args.root.join("daemon.toml"), config_text.as_bytes())?;
    milkdrift_daemon::DaemonConfig::load(&args.root.join("daemon.toml"))?;
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"recipe":reference,"installation":installation,"config":args.root.join("daemon.toml"),"credential_file":token_path,"platform_effects":"none; start this configuration, then use resource prepare/apply"})
        )?
    );
    Ok(())
}

fn private_directory(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(not(unix))]
    let _ = path;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        if !path.exists() {
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(path)?;
        }
        if fs::canonicalize(path)? != path
            || !fs::symlink_metadata(path)?.is_dir()
            || fs::metadata(path)?.permissions().mode() & 0o077 != 0
        {
            return Err("bootstrap requires canonical private directories without symlinks".into());
        }
    }
    Ok(())
}
fn write_exact(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if !metadata.is_file()
            || fs::canonicalize(path)? != path
            || metadata.len() != bytes.len() as u64
            || fs::read(path)? != bytes
        {
            return Err("bootstrap refuses to overwrite different existing files".into());
        }
        return Ok(());
    }
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_repeat_and_conflicting_bootstrap_use_the_production_config_reader()
    -> Result<(), Box<dyn std::error::Error>> {
        if !cfg!(target_os = "linux") {
            return Ok(());
        }
        let directory = tempfile::tempdir()?;
        let recipe = directory.path().join("approved.json");
        fs::write(
            &recipe,
            include_bytes!("../../../examples/managed-linux/slotbook.json"),
        )?;
        let root = directory.path().join("product");
        let quadlet = directory.path().join("quadlets");
        let args = |preview| Arguments {
            root: root.clone(),
            quadlet_directory: quadlet.clone(),
            systemd_directory: directory.path().join("systemd"),
            recipe: recipe.clone(),
            installation: None,
            preview,
        };
        run(args(true))?;
        assert!(!root.exists());
        assert!(!quadlet.exists());
        run(args(false))?;
        let token = fs::read(root.join("operator.token"))?;
        let config = fs::read(root.join("daemon.toml"))?;
        run(args(false))?;
        assert_eq!(fs::read(root.join("operator.token"))?, token);
        assert_eq!(fs::read(root.join("daemon.toml"))?, config);
        let mut changed: serde_json::Value = serde_json::from_slice(&fs::read(&recipe)?)?;
        changed["worker_network"] = json!("outbound");
        fs::write(&recipe, serde_json::to_vec(&changed)?)?;
        assert!(run(args(false)).is_err());
        assert_eq!(fs::read(root.join("daemon.toml"))?, config);
        Ok(())
    }
}
