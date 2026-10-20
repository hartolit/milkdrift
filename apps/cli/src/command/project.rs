use crate::{
    error::CliError,
    project_args::{EnvironmentArgs, EnvironmentKind, ProjectArgs, ProjectCommand},
    session::CliSession,
};
use milkdrift_control_protocol::projects::{
    ProjectCommandDto, ProjectCommandRequest, ProjectEnvironmentDto, ProjectSettingsDto,
};

pub(super) async fn execute(session: &CliSession, args: &ProjectArgs) -> Result<(), CliError> {
    let client = session.client();
    let command = match &args.command {
        ProjectCommand::List { limit, cursor } => {
            return session.output(
                "project.list",
                &client.projects(cursor.as_deref(), *limit).await?,
            );
        }
        ProjectCommand::Show { project } => {
            return session.output("project.show", &client.project(project).await?);
        }
        ProjectCommand::Directories {
            parent,
            cursor,
            limit,
        } => {
            return session.output(
                "project.directories",
                &client
                    .project_directories(parent.as_deref(), cursor.as_deref(), *limit)
                    .await?,
            );
        }
        ProjectCommand::Environments => {
            return session.output(
                "project.environments",
                &client.project_environments().await?,
            );
        }
        ProjectCommand::Receipt { request } => {
            return session.output("project.receipt", &client.project_receipt(request).await?);
        }
        ProjectCommand::Operations => {
            return session.output("project.operations", &client.project_operations().await?);
        }
        ProjectCommand::Attach { name, folder } => ProjectCommandDto::Attach {
            name: name.clone(),
            folder: folder.clone(),
            settings: initial_settings(),
        },
        ProjectCommand::Create {
            name,
            parent,
            folder_name,
        } => ProjectCommandDto::Create {
            name: name.clone(),
            parent: parent.clone(),
            folder_name: folder_name.clone(),
            settings: initial_settings(),
        },
        ProjectCommand::Rename {
            project,
            name,
            expected_generation,
        } => ProjectCommandDto::Rename {
            project_id: project.clone(),
            expected_generation: *expected_generation,
            name: name.clone(),
        },
        ProjectCommand::Settings {
            project,
            expected_generation,
            settings,
        } => ProjectCommandDto::Settings {
            project_id: project.clone(),
            expected_generation: *expected_generation,
            settings: environment_settings(settings)?,
        },
        ProjectCommand::Remove {
            project,
            expected_generation,
        } => ProjectCommandDto::Remove {
            project_id: project.clone(),
            expected_generation: *expected_generation,
        },
        ProjectCommand::Associate {
            project,
            workflow,
            expected_generation,
        } => ProjectCommandDto::Associate {
            project_id: project.clone(),
            expected_generation: *expected_generation,
            workflow_id: workflow.clone(),
        },
        ProjectCommand::Reconcile { operation } => ProjectCommandDto::Reconcile {
            operation_id: operation.clone(),
        },
        ProjectCommand::Abandon { operation } => ProjectCommandDto::Abandon {
            operation_id: operation.clone(),
        },
    };
    let request = ProjectCommandRequest {
        request_id: session
            .cli()
            .command_id
            .clone()
            .unwrap_or_else(crate::session::generated_command_id),
        command,
    };
    session.output("project.command", &client.project_command(&request).await?)
}

fn initial_settings() -> ProjectSettingsDto {
    ProjectSettingsDto {
        environment: ProjectEnvironmentDto::Host,
        allow_changes: false,
    }
}

fn environment_settings(args: &EnvironmentArgs) -> Result<ProjectSettingsDto, CliError> {
    let environment = match args.environment {
        EnvironmentKind::Host => {
            if args.capability.is_some()
                || args.operation.is_some()
                || args.environment_generation.is_some()
            {
                return Err(CliError::Invalid(
                    "host file tools accept no process capability or generation".into(),
                ));
            }
            ProjectEnvironmentDto::Host
        }
        EnvironmentKind::Process | EnvironmentKind::Podman => {
            let capability = args.capability.clone().ok_or_else(|| {
                CliError::Invalid("configured environments require --capability".into())
            })?;
            let operation = args.operation.clone().ok_or_else(|| {
                CliError::Invalid("configured environments require --operation".into())
            })?;
            let generation = args.environment_generation.ok_or_else(|| {
                CliError::Invalid("configured environments require --environment-generation".into())
            })?;
            if matches!(args.environment, EnvironmentKind::Process) {
                ProjectEnvironmentDto::Process {
                    capability,
                    operation,
                    generation,
                }
            } else {
                ProjectEnvironmentDto::Podman {
                    capability,
                    operation,
                    generation,
                }
            }
        }
    };
    Ok(ProjectSettingsDto {
        environment,
        allow_changes: args.allow_changes,
    })
}
