use std::collections::BTreeSet;

use cargo_metadata::Metadata;

use super::{CUDA_FEATURE, package_responsibility, package_role};

pub(crate) struct WorkspaceInventoryIssue {
    pub(crate) package: String,
    pub(crate) target: String,
    pub(crate) reason: String,
}

pub(crate) fn workspace_package_inventory(
    metadata: &Metadata,
) -> Result<Vec<String>, Vec<WorkspaceInventoryIssue>> {
    let mut inventory = BTreeSet::new();
    let mut issues = Vec::new();

    for package in metadata.workspace_packages() {
        let package_name = package.name.to_string();
        if !inventory.insert(package_name.clone()) {
            issues.push(WorkspaceInventoryIssue {
                package: package_name.clone(),
                target: "package identity".to_owned(),
                reason: "workspace package names must be unique for exact Cargo -p ownership"
                    .to_owned(),
            });
        }
        if let Err(error) = package_role(package) {
            issues.push(WorkspaceInventoryIssue {
                package: package_name,
                target: "role".to_owned(),
                reason: error.reason,
            });
        }
        if let Err(error) = package_responsibility(package) {
            issues.push(WorkspaceInventoryIssue {
                package: package.name.to_string(),
                target: "responsibility".to_owned(),
                reason: error.reason,
            });
        }
    }

    if inventory.is_empty() {
        issues.push(WorkspaceInventoryIssue {
            package: "workspace".to_owned(),
            target: "native verification ownership".to_owned(),
            reason: "at least one workspace package must declare a Milkdrift role".to_owned(),
        });
    }

    if issues.is_empty() {
        Ok(inventory.into_iter().collect())
    } else {
        Err(issues)
    }
}

pub(crate) fn domain_package_inventory(
    metadata: &Metadata,
) -> Result<Vec<String>, Vec<WorkspaceInventoryIssue>> {
    let mut inventory = BTreeSet::new();
    let mut package_names = BTreeSet::new();
    let mut issues = Vec::new();

    for package in metadata.workspace_packages() {
        let package_name = package.name.to_string();
        if !package_names.insert(package_name.clone()) {
            issues.push(WorkspaceInventoryIssue {
                package: package_name.clone(),
                target: "package identity".to_owned(),
                reason: "workspace package names must be unique for exact Cargo -p ownership"
                    .to_owned(),
            });
        }
        match package_role(package) {
            Ok(role) if role.is_domain() => {
                inventory.insert(package_name);
            }
            Ok(_) => {}
            Err(error) => issues.push(WorkspaceInventoryIssue {
                package: package_name,
                target: "role".to_owned(),
                reason: error.reason,
            }),
        }
        if let Err(error) = package_responsibility(package) {
            issues.push(WorkspaceInventoryIssue {
                package: package.name.to_string(),
                target: "responsibility".to_owned(),
                reason: error.reason,
            });
        }
    }

    if inventory.is_empty() {
        issues.push(WorkspaceInventoryIssue {
            package: "workspace".to_owned(),
            target: "portable domain ownership".to_owned(),
            reason: "at least one workspace package must declare a Milkdrift domain role"
                .to_owned(),
        });
    }

    if issues.is_empty() {
        Ok(inventory.into_iter().collect())
    } else {
        Err(issues)
    }
}

pub(crate) fn cuda_feature_package_inventory(
    metadata: &Metadata,
) -> Result<Vec<String>, Vec<WorkspaceInventoryIssue>> {
    let mut inventory = BTreeSet::new();
    let mut issues = Vec::new();

    for package in metadata
        .workspace_packages()
        .into_iter()
        .filter(|package| package.features.contains_key(CUDA_FEATURE))
    {
        let package_name = package.name.to_string();
        if !inventory.insert(package_name.clone()) {
            issues.push(WorkspaceInventoryIssue {
                package: package_name,
                target: CUDA_FEATURE.to_owned(),
                reason: "workspace package names must be unique for exact Cargo -p ownership"
                    .to_owned(),
            });
        }
    }

    if inventory.is_empty() {
        issues.push(WorkspaceInventoryIssue {
            package: "workspace".to_owned(),
            target: CUDA_FEATURE.to_owned(),
            reason: "at least one workspace package must declare the exact `cuda` feature"
                .to_owned(),
        });
    }

    if issues.is_empty() {
        Ok(inventory.into_iter().collect())
    } else {
        Err(issues)
    }
}
