//! Live folder pages reveal only currently authorized directory choices.
use super::{
    ActorSession, Owner, PublicFailure, checked_limit, decode_cursor, encode_cursor, invalid,
};
use milkdrift_authority::AuthorityOperation;
use milkdrift_control_protocol::projects::{ProjectDirectoryDto, ProjectDirectoryPageDto};
use std::path::Path;

impl Owner {
    pub(super) fn project_directories(
        &self,
        session: &ActorSession,
        parent: Option<&str>,
        cursor: Option<&str>,
        limit: u16,
    ) -> Result<ProjectDirectoryPageDto, PublicFailure> {
        checked_limit(limit)?;
        let Some(parent) = parent else {
            let mut items = Vec::new();
            for scope in &session.grant.resources().filesystem {
                if let Ok(root) = self.capture_project(
                    session,
                    Path::new(scope.root()),
                    AuthorityOperation::InspectProject,
                    false,
                ) {
                    let can_create = self
                        .authorize_project_root(
                            session,
                            &root,
                            AuthorityOperation::ManageProject,
                            true,
                        )
                        .is_ok();
                    let path = root.requested.to_string_lossy().into_owned();
                    items.push(ProjectDirectoryDto {
                        name: root
                            .requested
                            .file_name()
                            .map_or_else(|| path.clone(), |s| s.to_string_lossy().into_owned()),
                        path,
                        can_create,
                    });
                }
            }
            items.sort_by(|a, b| a.path.cmp(&b.path));
            items.dedup_by(|a, b| a.path == b.path);
            let feed = format!("project-directory-roots:{}", self.host_id);
            let start = decode_cursor(session, &feed, cursor)?
                .map(|value| {
                    value
                        .parse::<usize>()
                        .map_err(|_| invalid("invalid root continuation"))
                })
                .transpose()?
                .unwrap_or(0);
            if start > items.len() {
                return Err(invalid("root choices changed; refresh the chooser"));
            }
            let end = start
                .checked_add(usize::from(limit))
                .ok_or_else(|| invalid("root page overflow"))?
                .min(items.len());
            let next_cursor = (end < items.len())
                .then(|| encode_cursor(session, &feed, &end.to_string()))
                .transpose()?;
            let items = items
                .into_iter()
                .skip(start)
                .take(usize::from(limit))
                .collect();
            return Ok(ProjectDirectoryPageDto {
                parent: None,
                items,
                next_cursor,
            });
        };
        let root = self.capture_project(
            session,
            Path::new(parent),
            AuthorityOperation::InspectProject,
            false,
        )?;
        let query = serde_json::to_vec(&(parent, &root.identity))
            .map_err(|_| invalid("invalid folder choice"))?;
        let feed = format!(
            "project-directories:{}:{}",
            self.host_id,
            blake3::hash(&query)
        );
        let start = decode_cursor(session, &feed, cursor)?
            .map(|v| {
                v.parse::<usize>()
                    .map_err(|_| invalid("invalid folder continuation"))
            })
            .transpose()?
            .unwrap_or(0);
        if start > 65_536 {
            return Err(invalid(
                "folder enumeration exceeds the 65,536-entry boundary; select a narrower permitted parent",
            ));
        }
        let entries =
            std::fs::read_dir(&root.physical).map_err(|_| invalid("this folder cannot be read"))?;
        let mut items = Vec::new();
        let mut next_cursor = None;
        // Read-dir order is live, not a snapshot. Each page inspects at most 1,024 entries;
        // the total cursor is bounded so hostile directories cannot cause unlimited scans.
        for (index, entry) in entries.enumerate().skip(start).take(1_025) {
            if index >= 65_536 {
                return Err(invalid(
                    "folder enumeration exceeds the 65,536-entry boundary; select a narrower permitted parent",
                ));
            }
            if index - start == 1_024 || items.len() == usize::from(limit) {
                next_cursor = Some(encode_cursor(session, &feed, &index.to_string())?);
                break;
            }
            let Ok(entry) = entry else {
                continue;
            };
            let path = root.requested.join(entry.file_name());
            let Ok(child) =
                self.capture_project(session, &path, AuthorityOperation::InspectProject, false)
            else {
                continue;
            };
            // A symlink to another allowed folder is visible, but gets its actual identity
            // at attachment; a symlink outside current authority is filtered above.
            let can_create = self
                .authorize_project_root(session, &child, AuthorityOperation::ManageProject, true)
                .is_ok();
            items.push(ProjectDirectoryDto {
                path: path.to_string_lossy().into_owned(),
                name: entry.file_name().to_string_lossy().into_owned(),
                can_create,
            });
        }
        super::revalidate(&root)?;
        Ok(ProjectDirectoryPageDto {
            parent: Some(parent.into()),
            items,
            next_cursor,
        })
    }
}
