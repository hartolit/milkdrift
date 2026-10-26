//! Regenerate the standalone client's transport types from their Rust owner.
use milkdrift_control_protocol::authoring::{
    AuthoringBoundsDto, AuthoringConnectionDto, AuthoringConnectionKindDto, AuthoringDocumentDto,
    AuthoringIssueDto, AuthoringModelDto, AuthoringReviewDto, AuthoringSnapshotDto,
    AuthoringSourceDto, AuthoringTaskDto, CanvasPositionDto, CheckpointAuthoringRequest,
    PositionsSnapshotDto, PutPositionsRequest, ReviewAuthoringRequest, RunAuthoringRequest,
};
use milkdrift_control_protocol::projects::{
    ProjectCommandDto, ProjectCommandRequest, ProjectDirectoryDto, ProjectDirectoryPageDto,
    ProjectDto, ProjectEnvironmentChoiceDto, ProjectEnvironmentDto, ProjectEnvironmentPageDto,
    ProjectPageDto, ProjectReceiptDto, ProjectSettingsDto,
};
use milkdrift_control_protocol::task_execution::{
    TaskExecutionDto, TaskProjectAccessDto, TaskProjectBindingDto, TaskProjectFolderDto,
};
use milkdrift_control_protocol::tasks::{
    ChangeWorkflowRequest, CreateWorkflowRequest, FrozenInputDto, FrozenTaskDto, InputBinding,
    InputValue, ModelAdmissionDto, NewTaskDto, PathAccess, TaskActionPolicyDto,
    TaskBindingRedirectDto, TaskCapability, TaskCapabilityAuthoringDto, TaskChangeDto,
    TaskDefinitionDto, TaskDependenciesDto, TaskInputMediaDto, TaskInputSlotDto,
    TaskOrderConditionDto, TaskOutputDeclarationDto, TaskOutputPortDto, TaskProjectRootDto,
    TaskPublishedOutputDto, TaskReadDto, TaskResultDto, TaskSessionDto, TaskStateDto,
    TaskSummaryDto, TaskSuppliedInputDto, TaskToolDto, TaskUploadReceipt, TaskUploadRequest,
    WorkflowLimitsDto, WorkflowPageDto, WorkflowReadDto, WorkflowReceipt, WorkflowSummaryDto,
};
use milkdrift_control_protocol::topology::{
    TopologyInputDto, TopologyTaskDto, WorkflowTopologyDto,
};
use milkdrift_control_protocol::{DaemonMetadataDto, RenameDaemonReceipt, RenameDaemonRequest};
use ts_rs::TS;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = ts_rs::Config::default();
    let declarations = [
        TopologyInputDto::decl(&config),
        TopologyTaskDto::decl(&config),
        WorkflowTopologyDto::decl(&config),
        AuthoringBoundsDto::decl(&config),
        AuthoringModelDto::decl(&config),
        AuthoringTaskDto::decl(&config),
        AuthoringSourceDto::decl(&config),
        AuthoringConnectionKindDto::decl(&config),
        AuthoringConnectionDto::decl(&config),
        AuthoringDocumentDto::decl(&config),
        AuthoringSnapshotDto::decl(&config),
        CheckpointAuthoringRequest::decl(&config),
        CanvasPositionDto::decl(&config),
        PositionsSnapshotDto::decl(&config),
        PutPositionsRequest::decl(&config),
        ReviewAuthoringRequest::decl(&config),
        AuthoringIssueDto::decl(&config),
        AuthoringReviewDto::decl(&config),
        RunAuthoringRequest::decl(&config),
        ProjectEnvironmentDto::decl(&config),
        ProjectSettingsDto::decl(&config),
        ProjectDto::decl(&config),
        ProjectPageDto::decl(&config),
        ProjectDirectoryDto::decl(&config),
        ProjectDirectoryPageDto::decl(&config),
        ProjectEnvironmentChoiceDto::decl(&config),
        ProjectEnvironmentPageDto::decl(&config),
        ProjectCommandDto::decl(&config),
        ProjectCommandRequest::decl(&config),
        ProjectReceiptDto::decl(&config),
        TaskExecutionDto::decl(&config),
        TaskProjectBindingDto::decl(&config),
        TaskProjectFolderDto::decl(&config),
        TaskProjectAccessDto::decl(&config),
        InputValue::decl(&config),
        InputBinding::decl(&config),
        TaskCapability::decl(&config),
        TaskInputSlotDto::decl(&config),
        TaskOutputPortDto::decl(&config),
        TaskCapabilityAuthoringDto::decl(&config),
        TaskDependenciesDto::decl(&config),
        TaskOrderConditionDto::decl(&config),
        TaskInputMediaDto::decl(&config),
        TaskProjectRootDto::decl(&config),
        TaskPublishedOutputDto::decl(&config),
        TaskSuppliedInputDto::decl(&config),
        PathAccess::decl(&config),
        TaskOutputDeclarationDto::decl(&config),
        TaskBindingRedirectDto::decl(&config),
        TaskActionPolicyDto::decl(&config),
        TaskToolDto::decl(&config),
        TaskDefinitionDto::decl(&config),
        NewTaskDto::decl(&config),
        WorkflowLimitsDto::decl(&config),
        CreateWorkflowRequest::decl(&config),
        TaskChangeDto::decl(&config),
        ChangeWorkflowRequest::decl(&config),
        WorkflowReceipt::decl(&config),
        TaskStateDto::decl(&config),
        TaskResultDto::decl(&config),
        FrozenInputDto::decl(&config),
        FrozenTaskDto::decl(&config),
        TaskReadDto::decl(&config),
        TaskSummaryDto::decl(&config),
        WorkflowSummaryDto::decl(&config),
        ModelAdmissionDto::decl(&config),
        WorkflowReadDto::decl(&config),
        WorkflowPageDto::decl(&config),
        TaskSessionDto::decl(&config),
        DaemonMetadataDto::decl(&config),
        RenameDaemonRequest::decl(&config),
        RenameDaemonReceipt::decl(&config),
        TaskUploadRequest::decl(&config),
        TaskUploadReceipt::decl(&config),
    ];
    let mut output = String::from(
        "// Generated by cargo run -p milkdrift-control-protocol --example export_web --features web-codegen.\n// Edit the owning Rust DTOs, then regenerate.\n",
    );
    output.push_str(&format!(
        "export const PROTOCOL = {{ major: {}, minor: {} }} as const;\n",
        milkdrift_control_protocol::ProtocolVersion::CURRENT.major,
        milkdrift_control_protocol::ProtocolVersion::CURRENT.minor
    ));
    for declaration in declarations {
        output.push_str("export ");
        output.push_str(&declaration);
        output.push('\n');
    }
    output.push_str("\nexport function decodeResponse<T>(body: unknown): T {\n  if (typeof body !== 'object' || body === null) throw new Error('Invalid daemon response');\n  const envelope = body as { protocol?: { major?: number; minor?: number }; value?: unknown };\n  if (envelope.protocol?.major !== PROTOCOL.major || envelope.protocol?.minor !== PROTOCOL.minor || !('value' in envelope)) throw new Error('Incompatible daemon protocol');\n  return envelope.value as T;\n}\n");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/web/src/lib/protocol.generated.ts");
    std::fs::create_dir_all(path.parent().ok_or("generated output has no parent")?)?;
    let normalized = output
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(path, format!("{normalized}\n"))?;
    Ok(())
}
