//! A bounded conversation inside one entered task; this owner never schedules tasks.

use crate::{
    AdapterError, AdapterExecutionContext, AdapterInvocation, AdapterReporter, CapabilityAdapter,
    CapabilityHost, InvocationDataAccess, MaterializationLimits, PreparedAdapterExecution,
};
use milkdrift_authority::{AuthorityEvaluator, CapabilityExecutionRequirements};
use milkdrift_blueprint::TaskTool;
use milkdrift_capability::{
    BoundedJson, CancellationAcknowledgement, CancellationRequest, CapabilityCategory,
    CapabilityDescriptor, CapabilityObservation, InputReference, InvocationAdmissionEnvelope,
    InvocationId, InvocationRequest, InvocationValueReference, TerminalStatus,
};
use milkdrift_model::{
    ContentPart, MODEL_TASK_INPUT_NAME, Message, MessageRole, ModelResponseDocument,
    ModelTaskRequest, ModelTaskRequestDocument, ToolDefinition,
};
use milkdrift_runtime::{CapabilityResolutionContext, ResolvedCapability, TaskExecutor};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

mod files;
mod reporting;
use reporting::{LoopReports, RoundReports};

// Six reports per response plus one receipt per tool fit the task owner's 256 report slots.
const MAX_TOOL_CALLS: u32 = 16;
const FEEDBACK_BYTES: u64 = 32_768;

pub(super) fn limits() -> MaterializationLimits {
    MaterializationLimits {
        max_files: 256,
        max_file_bytes: 2_097_152,
        max_total_bytes: 2_097_152,
        max_path_bytes: 4096,
        max_directory_depth: 64,
        chunk_bytes: 1_048_576,
    }
}

struct ActiveLoop {
    stopped: Arc<AtomicBool>,
    process: Option<InvocationId>,
}

/// Runs explicitly admitted tools between requests to one configured model generation.
/// The inner adapter remains the sole provider mapping. Every subentry rechecks current
/// authority and stays within the allowance reserved before the outer task enters.
pub struct ModelToolLoopAdapter {
    descriptor: CapabilityDescriptor,
    model: Arc<dyn CapabilityAdapter>,
    maximum_model: InvocationAdmissionEnvelope,
    host: CapabilityHost,
    data: Arc<dyn InvocationDataAccess>,
    authority: Arc<dyn AuthorityEvaluator>,
    clock: Arc<dyn Fn() -> Result<u64, AdapterError> + Send + Sync>,
    active: Mutex<BTreeMap<InvocationId, ActiveLoop>>,
}

struct ProcessTool {
    name: String,
    resolved: ResolvedCapability,
    maximum: InvocationAdmissionEnvelope,
}
struct PreparedLoop {
    request: InvocationRequest,
    prepared: PreparedAdapterExecution,
    processes: Vec<ProcessTool>,
    maximum_calls: u32,
}

impl ModelToolLoopAdapter {
    /// Composes an already configured provider adapter with the daemon's host and authority.
    /// No tool is exposed unless the frozen task definition names it.
    #[must_use]
    pub fn new(
        descriptor: CapabilityDescriptor,
        model: Arc<dyn CapabilityAdapter>,
        maximum_model: InvocationAdmissionEnvelope,
        host: CapabilityHost,
        data: Arc<dyn InvocationDataAccess>,
        authority: Arc<dyn AuthorityEvaluator>,
        clock: Arc<dyn Fn() -> Result<u64, AdapterError> + Send + Sync>,
    ) -> Self {
        Self {
            descriptor,
            model,
            maximum_model,
            host,
            data,
            authority,
            clock,
            active: Mutex::new(BTreeMap::new()),
        }
    }

    fn prepare_loop(
        &self,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<(InvocationAdmissionEnvelope, PreparedLoop), AdapterError> {
        let context = invocation
            .context()
            .ok_or_else(|| rejected("model tools require an entered task context"))?;
        if context.maximum_tool_calls == 0 || context.maximum_tool_calls > MAX_TOOL_CALLS {
            return Err(rejected(
                "model tool calls must be within the host's 1..=16 durable-report ceiling",
            ));
        }
        let mut task = read_task(self.data.as_ref(), context, invocation.request())?;
        if !task.tools().is_empty() || task.streaming() {
            return Err(rejected(
                "host model tools require an initially empty catalog and non-streaming bounded responses",
            ));
        }
        let resolution_context = resolution_context(context)?;
        let now = (self.clock)()?;
        let mut definitions = Vec::new();
        let mut processes = Vec::new();
        for (index, tool) in context.task_tools.iter().enumerate() {
            match tool {
                TaskTool::ReadFile | TaskTool::WriteFile => {
                    definitions.push(files::definition(tool)?)
                }
                TaskTool::Process { capability } => {
                    let resolved = self
                        .host
                        .resolve_authorized(
                            capability,
                            &resolution_context,
                            self.authority.as_ref(),
                            now,
                        )
                        .map_err(adapter_error)?;
                    if resolved.descriptor().category() != &CapabilityCategory::Process {
                        return Err(rejected(
                            "process tools must resolve to an ordinary process capability",
                        ));
                    }
                    let name = format!("process_{index}");
                    let adapter = self.host.loop_adapter(resolved.snapshot())?;
                    let request = process_request(
                        invocation.request(),
                        resolved.snapshot(),
                        u32::try_from(index).map_err(adapter_error)?,
                        Vec::new(),
                    )?;
                    let selected_context =
                        context.for_model_request(&request).map_err(adapter_error)?;
                    let maximum = adapter.admission_envelope(&AdapterInvocation::with_context(
                        resolved.snapshot(),
                        &request,
                        &selected_context,
                    ))?;
                    definitions.push(ToolDefinition::new(&name, format!("Run configured process {} using named JSON inputs; its operator profile controls executable and arguments.", resolved.snapshot().capability()), BoundedJson::new(serde_json::json!({"type":"object","properties":{"inputs":{"type":"object"}},"required":["inputs"],"additionalProperties":false})).map_err(adapter_error)?).map_err(adapter_error)?);
                    processes.push(ProcessTool {
                        name,
                        resolved,
                        maximum,
                    });
                }
            }
        }
        task = task_with(&task, task.messages().to_vec(), definitions)?;
        let request = model_request(invocation.request(), task)?;
        let selected_context = context.for_model_request(&request).map_err(adapter_error)?;
        let prepared = self
            .model
            .clone()
            .prepare(&AdapterInvocation::with_context(
                invocation.resolution(),
                &request,
                &selected_context,
            ))?;
        let envelope = reporting::reserve(
            &self.maximum_model,
            &processes
                .iter()
                .map(|tool| &tool.maximum)
                .collect::<Vec<_>>(),
            context.maximum_tool_calls,
        )?;
        reporting::fits(prepared.envelope(), &self.maximum_model)?;
        Ok((
            envelope,
            PreparedLoop {
                request,
                prepared,
                processes,
                maximum_calls: context.maximum_tool_calls,
            },
        ))
    }

    fn run(
        &self,
        invocation: &AdapterInvocation<'_>,
        prepared_loop: PreparedLoop,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        let context = invocation
            .context()
            .ok_or_else(|| rejected("model task context is missing"))?;
        let stopped = Arc::new(AtomicBool::new(false));
        if self
            .active
            .lock()
            .map_err(|_| rejected("model loop owner unavailable"))?
            .insert(
                invocation.request().invocation().clone(),
                ActiveLoop {
                    stopped: stopped.clone(),
                    process: None,
                },
            )
            .is_some()
        {
            return Err(rejected("model loop already active"));
        }
        let result = self.run_active(invocation, context, prepared_loop, reporter, &stopped);
        self.active
            .lock()
            .map_err(|_| rejected("model loop owner unavailable"))?
            .remove(invocation.request().invocation());
        result
    }

    fn run_active(
        &self,
        invocation: &AdapterInvocation<'_>,
        context: &AdapterExecutionContext,
        prepared_loop: PreparedLoop,
        reporter: &dyn AdapterReporter,
        stopped: &AtomicBool,
    ) -> Result<(), AdapterError> {
        let mut request = prepared_loop.request;
        let mut prepared = prepared_loop.prepared;
        let mut reports =
            LoopReports::new(invocation.request(), context, reporter, self.data.as_ref());
        let mut calls = 0_u32;
        loop {
            if stopped.load(Ordering::SeqCst) {
                return reports.cancelled();
            }
            if let Err(error) = self.authorize(
                context,
                &self.descriptor,
                invocation.request().operation(),
                &self.model.authority_requirements(),
                "model-loop",
            ) {
                return reports.failure(error.summary());
            }
            let selected_context = context.for_model_request(&request).map_err(adapter_error)?;
            let round = RoundReports::new(&reports, &request, &selected_context);
            prepared.enter(
                &AdapterInvocation::with_context(
                    invocation.resolution(),
                    &request,
                    &selected_context,
                ),
                &round,
            )?;
            let observed = round.finish()?;
            reports.observe(&observed.terminal, true)?;
            if observed.terminal.status() != TerminalStatus::Success {
                return reports.finish(observed);
            }
            let reference = observed.outputs.get("model_response").ok_or_else(|| {
                AdapterError::response_observed_failure(
                    "provider completed without canonical model response",
                )
            })?;
            let bytes = read_output(self.data.as_ref(), context, &request, reference)?;
            let response = ModelResponseDocument::from_json(&bytes)
                .map_err(adapter_error)?
                .body()
                .clone();
            if response.tool_calls().is_empty() {
                return reports.finish(observed);
            }
            let additional = u32::try_from(response.tool_calls().len()).map_err(adapter_error)?;
            if calls
                .checked_add(additional)
                .is_none_or(|count| count > prepared_loop.maximum_calls)
            {
                return reports
                    .failure("model requested more tool calls than the frozen task allowance");
            }
            let task = read_task(self.data.as_ref(), &selected_context, &request)?;
            let mut messages = task.messages().to_vec();
            messages.push(
                Message::new(
                    MessageRole::Assistant,
                    vec![ContentPart::Text {
                        text: response.text().to_owned(),
                    }],
                    None,
                )
                .and_then(|message| message.with_tool_calls(response.tool_calls().to_vec()))
                .map_err(adapter_error)?,
            );
            for call in response.tool_calls() {
                if stopped.load(Ordering::SeqCst) {
                    return reports.cancelled();
                }
                if let Err(error) = self.authorize(
                    context,
                    &self.descriptor,
                    invocation.request().operation(),
                    &self.model.authority_requirements(),
                    "tool-loop",
                ) {
                    return reports.failure(error.summary());
                }
                calls = calls
                    .checked_add(1)
                    .ok_or_else(|| rejected("tool count overflow"))?;
                let feedback = match call.name() {
                    "read_file" if context.task_tools.contains(&TaskTool::ReadFile) => files::read(
                        &self.host.model_path_roots(context)?,
                        call.arguments().value(),
                    ),
                    "write_file" if context.task_tools.contains(&TaskTool::WriteFile) => {
                        files::write(
                            &self.host.model_path_roots(context)?,
                            call.arguments().value(),
                        )
                    }
                    name => match prepared_loop
                        .processes
                        .iter()
                        .find(|tool| tool.name == name)
                    {
                        Some(tool) => self.process(
                            invocation,
                            context,
                            tool,
                            call.arguments().value(),
                            calls,
                            &mut reports,
                        ),
                        None => {
                            return reports
                                .failure("model selected a tool outside the admitted catalog");
                        }
                    },
                };
                let feedback = match feedback {
                    Ok(value) => value,
                    Err(error) if error.kind() == crate::AdapterFailureKind::ExternalFailure => {
                        return Err(error);
                    }
                    Err(error) => return reports.failure(error.summary()),
                };
                let feedback = if call.name() == "write_file" {
                    let text = call
                        .arguments()
                        .value()
                        .get("text")
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| rejected("validated file content disappeared"))?;
                    let reference = reports.file(calls, text)?;
                    serde_json::to_string(&serde_json::json!({"written":serde_json::from_str::<serde_json::Value>(&feedback).map_err(adapter_error)?,"output":format!("file_{calls}"),"reference":reference})).map_err(adapter_error)?
                } else {
                    feedback
                };
                if feedback.len() as u64 > FEEDBACK_BYTES {
                    return reports.failure("tool result exceeds the bounded feedback ceiling");
                }
                reports.tool(call, &feedback)?;
                messages.push(
                    Message::new(
                        MessageRole::ToolResult,
                        vec![ContentPart::Text { text: feedback }],
                        Some(call.id().to_owned()),
                    )
                    .map_err(adapter_error)?,
                );
            }
            let next_task = task_with(&task, messages, task.tools().to_vec())?;
            request = model_request(invocation.request(), next_task)?;
            let selected_context = context.for_model_request(&request).map_err(adapter_error)?;
            prepared = match self.model.clone().prepare(&AdapterInvocation::with_context(
                invocation.resolution(),
                &request,
                &selected_context,
            )) {
                Ok(prepared) => prepared,
                Err(error) => return reports.failure(error.summary()),
            };
            if let Err(error) = reporting::fits(prepared.envelope(), &self.maximum_model) {
                return reports.failure(error.summary());
            }
        }
    }

    fn authorize(
        &self,
        context: &AdapterExecutionContext,
        descriptor: &CapabilityDescriptor,
        operation: &milkdrift_capability::OperationId,
        requirements: &CapabilityExecutionRequirements,
        boundary: &str,
    ) -> Result<(), AdapterError> {
        self.host.revalidate_model_paths(context)?;
        let mut requirements = requirements.clone();
        for path in &context.task_paths {
            let access = if path.write {
                std::collections::BTreeSet::from([
                    milkdrift_authority::AccessMode::Read,
                    milkdrift_authority::AccessMode::Write,
                ])
            } else {
                std::collections::BTreeSet::from([milkdrift_authority::AccessMode::Read])
            };
            requirements.filesystem.push(
                milkdrift_authority::FilesystemScope::new(path.path.clone(), access)
                    .map_err(adapter_error)?,
            );
        }
        let request = resolution_context(context)?
            .candidate_request(
                descriptor,
                operation,
                &requirements,
                (self.clock)()?,
                boundary,
            )
            .map_err(adapter_error)?;
        let decision = self.authority.evaluate(&request).map_err(adapter_error)?;
        if !decision.is_allowed() || decision.request() != &request {
            return Err(rejected("current authority refused a model/tool entry"));
        }
        Ok(())
    }

    fn process(
        &self,
        invocation: &AdapterInvocation<'_>,
        context: &AdapterExecutionContext,
        tool: &ProcessTool,
        arguments: &serde_json::Value,
        call: u32,
        reports: &mut LoopReports<'_>,
    ) -> Result<String, AdapterError> {
        let object = arguments
            .as_object()
            .filter(|object| object.len() == 1)
            .ok_or_else(|| rejected("process tool requires exactly an inputs object"))?;
        let inputs = object
            .get("inputs")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| rejected("process tool inputs must be a named object"))?
            .iter()
            .map(|(name, value)| {
                InputReference::new(
                    name,
                    InvocationValueReference::Inline {
                        value: BoundedJson::new(value.clone()).map_err(adapter_error)?,
                    },
                )
                .map_err(adapter_error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let request =
            process_request(invocation.request(), tool.resolved.snapshot(), call, inputs)?;
        let selected_context = context.for_model_request(&request).map_err(adapter_error)?;
        let prepared = self
            .host
            .prepare_invocation(tool.resolved.snapshot(), &request, Some(&selected_context))
            .map_err(adapter_error)?;
        reporting::fits(prepared.envelope(), &tool.maximum)?;
        let adapter = self.host.loop_adapter(tool.resolved.snapshot())?;
        self.authorize(
            context,
            tool.resolved.descriptor(),
            request.operation(),
            &adapter.authority_requirements(),
            "process-tool",
        )?;
        self.active
            .lock()
            .map_err(|_| rejected("model loop owner unavailable"))?
            .get_mut(invocation.request().invocation())
            .ok_or_else(|| rejected("active model loop disappeared"))?
            .process = Some(request.invocation().clone());
        let round = RoundReports::new(reports, &request, &selected_context);
        self.host
            .verify_process_tool_claims(tool.resolved.snapshot(), &request, &selected_context)
            .map_err(adapter_error)?;
        prepared
            .enter(Some(&selected_context), &round)
            .map_err(|error| AdapterError::external_failure(error.to_string()))?;
        self.active
            .lock()
            .map_err(|_| rejected("model loop owner unavailable"))?
            .get_mut(invocation.request().invocation())
            .ok_or_else(|| rejected("active model loop disappeared"))?
            .process = None;
        let observed = round.finish()?;
        reports.observe(&observed.terminal, false)?;
        if observed.terminal.status() == TerminalStatus::Uncertain {
            return Err(AdapterError::external_failure(
                "process tool outcome remains uncertain",
            ));
        }
        let mut outputs = BTreeMap::new();
        for (name, reference) in &observed.outputs {
            let content = if reference
                .size_bytes()
                .is_some_and(|bytes| bytes <= FEEDBACK_BYTES)
            {
                String::from_utf8(read_output(
                    self.data.as_ref(),
                    context,
                    &request,
                    reference,
                )?)
                .ok()
            } else {
                None
            };
            outputs.insert(
                name,
                serde_json::json!({"reference": reference, "text": content}),
            );
        }
        serde_json::to_string(&serde_json::json!({"status": observed.terminal.status(), "failure": observed.terminal.failure(), "outputs": outputs})).map_err(adapter_error)
    }
}

impl CapabilityAdapter for ModelToolLoopAdapter {
    fn native_path_claims(
        &self,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<Vec<crate::NativePathScope>, AdapterError> {
        let context = invocation
            .context()
            .ok_or_else(|| rejected("model tools require task provenance"))?;
        let authority = resolution_context(context)?;
        let now = (self.clock)()?;
        let mut paths = Vec::new();
        for tool in &context.task_tools {
            if let TaskTool::Process { capability } = tool {
                let resolved = self
                    .host
                    .resolve_authorized(capability, &authority, self.authority.as_ref(), now)
                    .map_err(adapter_error)?;
                if resolved.descriptor().category() != &CapabilityCategory::Process {
                    return Err(rejected(
                        "native process scope requires a process capability",
                    ));
                }
                let request =
                    process_request(invocation.request(), resolved.snapshot(), 0, Vec::new())?;
                paths.extend(
                    self.host
                        .declared_native_paths(resolved.snapshot(), &request, context)
                        .map_err(adapter_error)?
                        .into_iter()
                        .map(|path| crate::NativePathScope {
                            path: path.path.into(),
                            write: path.write,
                        }),
                );
            }
        }
        Ok(paths)
    }

    fn task_authoring(
        &self,
    ) -> BTreeMap<milkdrift_capability::OperationId, crate::TaskAuthoringContract> {
        self.model.task_authoring()
    }

    fn accepts_direct_inputs(&self) -> bool {
        self.model.accepts_direct_inputs()
    }
    fn authority_requirements(&self) -> CapabilityExecutionRequirements {
        self.model.authority_requirements()
    }
    fn admission_envelope(
        &self,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<InvocationAdmissionEnvelope, AdapterError> {
        self.model.admission_envelope(invocation)
    }
    fn prepare(
        self: Arc<Self>,
        invocation: &AdapterInvocation<'_>,
    ) -> Result<PreparedAdapterExecution, AdapterError> {
        if invocation
            .context()
            .is_none_or(|context| context.task_tools.is_empty())
        {
            return self.model.clone().prepare(invocation);
        }
        let (envelope, prepared) = self.prepare_loop(invocation)?;
        Ok(PreparedAdapterExecution::new(
            envelope,
            move |invocation, reporter| self.run(invocation, prepared, reporter),
        ))
    }
    fn execute(
        &self,
        invocation: &AdapterInvocation<'_>,
        reporter: &dyn AdapterReporter,
    ) -> Result<(), AdapterError> {
        if invocation
            .context()
            .is_none_or(|context| context.task_tools.is_empty())
        {
            return self.model.execute(invocation, reporter);
        }
        let (_, prepared) = self.prepare_loop(invocation)?;
        self.run(invocation, prepared, reporter)
    }
    fn start(&self) -> Result<(), AdapterError> {
        self.model.start()
    }
    fn cancel(
        &self,
        request: &CancellationRequest,
    ) -> Result<CancellationAcknowledgement, AdapterError> {
        let active = self
            .active
            .lock()
            .map_err(|_| rejected("model loop owner unavailable"))?;
        let process = if let Some(active) = active.get(request.invocation()) {
            active.stopped.store(true, Ordering::SeqCst);
            active.process.clone()
        } else {
            None
        };
        drop(active);
        if let Some(process) = process {
            let cancellation = CancellationRequest::new(
                process,
                request.request_sequence(),
                request.reason().to_owned(),
            )
            .map_err(adapter_error)?;
            self.host
                .cancel_exact(&cancellation)
                .map_err(adapter_error)?;
        }
        self.model.cancel(request)
    }
    fn health(&self, now: u64) -> Result<CapabilityObservation, AdapterError> {
        self.model.health(now)
    }
    fn begin_drain(&self) -> Result<(), AdapterError> {
        self.model.begin_drain()
    }
    fn shutdown(&self) -> Result<(), AdapterError> {
        self.model.shutdown()
    }
}

fn resolution_context(
    context: &AdapterExecutionContext,
) -> Result<CapabilityResolutionContext, AdapterError> {
    let workflow = context
        .workflow()
        .ok_or_else(|| rejected("model tools require local task provenance"))?;
    Ok(CapabilityResolutionContext::new(
        context
            .authority()
            .ok_or_else(|| rejected("model tools require accepted authority"))?
            .clone(),
        workflow.revision().clone(),
        workflow.node().clone(),
        workflow.execution().clone(),
        workflow.attempt().clone(),
    )
    .with_project_binding(context.project_binding().cloned()))
}
fn task_with(
    task: &ModelTaskRequest,
    messages: Vec<Message>,
    tools: Vec<ToolDefinition>,
) -> Result<ModelTaskRequest, AdapterError> {
    ModelTaskRequest::new(
        messages,
        tools,
        task.structured_output().cloned(),
        task.session().clone(),
        task.reasoning(),
        task.maximum_output_units(),
        task.streaming(),
        task.extensions().clone(),
    )
    .map_err(adapter_error)
}
fn read_task(
    data: &dyn InvocationDataAccess,
    context: &AdapterExecutionContext,
    request: &InvocationRequest,
) -> Result<ModelTaskRequest, AdapterError> {
    let input = request
        .inputs()
        .iter()
        .find(|input| input.name() == MODEL_TASK_INPUT_NAME)
        .ok_or_else(|| rejected("model task document is missing"))?;
    let bytes = data
        .read_input_bytes(context, input, limits())
        .map_err(adapter_error)?;
    ModelTaskRequestDocument::from_json(&bytes)
        .map(|document| document.body().clone())
        .map_err(adapter_error)
}
fn model_request(
    request: &InvocationRequest,
    task: ModelTaskRequest,
) -> Result<InvocationRequest, AdapterError> {
    let mut inputs = request
        .inputs()
        .iter()
        .filter(|input| input.name() != MODEL_TASK_INPUT_NAME)
        .cloned()
        .collect::<Vec<_>>();
    inputs.push(
        InputReference::new(
            MODEL_TASK_INPUT_NAME,
            InvocationValueReference::Inline {
                value: BoundedJson::new(
                    serde_json::to_value(ModelTaskRequestDocument::new(task))
                        .map_err(adapter_error)?,
                )
                .map_err(adapter_error)?,
            },
        )
        .map_err(adapter_error)?,
    );
    let mut extensions = request.extensions().clone();
    extensions.insert(
        milkdrift_capability::ExtensionKey::new("org.milkdrift/model-tool-loop")
            .map_err(adapter_error)?,
        BoundedJson::new(serde_json::json!(true)).map_err(adapter_error)?,
    );
    InvocationRequest::new(
        request.invocation().clone(),
        request.capability().clone(),
        request.operation().clone(),
        request.provider_profile().cloned(),
        request.idempotency_key().cloned(),
        inputs,
        extensions,
    )
    .map_err(adapter_error)
}
fn process_request(
    parent: &InvocationRequest,
    snapshot: &milkdrift_capability::ResolvedCapabilitySnapshot,
    call: u32,
    inputs: Vec<InputReference>,
) -> Result<InvocationRequest, AdapterError> {
    InvocationRequest::new(
        milkdrift_persistence::managed::task_tool_invocation(parent.invocation(), call)
            .map_err(adapter_error)?,
        snapshot.capability().clone(),
        snapshot.operation().clone(),
        snapshot.provider_profile().cloned(),
        None,
        inputs,
        BTreeMap::new(),
    )
    .map_err(adapter_error)
}
fn read_output(
    data: &dyn InvocationDataAccess,
    context: &AdapterExecutionContext,
    request: &InvocationRequest,
    reference: &milkdrift_capability::ArtifactReference,
) -> Result<Vec<u8>, AdapterError> {
    let selection = InvocationRequest::new(
        request.invocation().clone(),
        request.capability().clone(),
        request.operation().clone(),
        request.provider_profile().cloned(),
        None,
        vec![
            InputReference::new(
                "observed_output",
                InvocationValueReference::Artifact {
                    reference: reference.clone(),
                },
            )
            .map_err(adapter_error)?,
        ],
        BTreeMap::new(),
    )
    .map_err(adapter_error)?;
    data.read_artifact_bytes(
        &context
            .for_model_request(&selection)
            .map_err(adapter_error)?,
        reference,
        limits(),
    )
    .map_err(adapter_error)
}
fn rejected(message: &str) -> AdapterError {
    AdapterError::rejected(message)
}
fn adapter_error(error: impl std::fmt::Display) -> AdapterError {
    AdapterError::rejected(error.to_string())
}
