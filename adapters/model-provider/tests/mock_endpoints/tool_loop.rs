//! Real HTTP turns and native file effects through the entered production task host.
use super::runtime_session::{Allow, ModelFixture, NoFaults};
use super::*;
use milkdrift_authority::{
    AuthorityBudget, AuthorityDecisionSnapshot, AuthorityError, AuthorityEvaluator,
    AuthorityRequest, DecisionReasonCode,
};
use milkdrift_blueprint::{TaskPathAccess, TaskTool};

fn endpoint(address: &str) -> TestResult<EndpointProfile> {
    let mut profile = serde_json::to_value(super::effect_stages::endpoint(address)?)?;
    *profile
        .get_mut("features")
        .ok_or("profile features absent")? = json!(["system_role", "tools", "structured_output"]);
    Ok(serde_json::from_value(profile)?)
}
fn tool(name: &str, arguments: Value) -> String {
    json!({"choices":[{"message":{"content":"","tool_calls":[{"id":"call_1","type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":2,"completion_tokens":3}}).to_string()
}
fn sequence(
    responses: Vec<String>,
    revoke: Option<Arc<AtomicBool>>,
) -> TestResult<(String, thread::JoinHandle<std::io::Result<Vec<String>>>)> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?.to_string();
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for body in responses {
            let started = std::time::Instant::now();
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && started.elapsed() < Duration::from_secs(5) =>
                    {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => return Err(error),
                }
            };
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            requests.push(read_request(&mut stream)?);
            if let Some(revoke) = &revoke {
                revoke.store(true, Ordering::SeqCst);
            }
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes())?;
        }
        Ok(requests)
    });
    Ok((address, server))
}

#[test]
fn ordinary_task_model_loop_writes_reads_and_retains_exact_feedback_and_usage() -> TestResult {
    let project = tempfile::tempdir()?;
    let path = project.path().join("research.txt");
    let text = "Native evidence: æ\nsecond line\n";
    let (address, server) = sequence(
        vec![
            tool("write_file", json!({"path":path,"text":text})),
            tool("read_file", json!({"path":path})),
            super::effect_stages::complete_response(),
        ],
        None,
    )?;
    let fixture = ModelFixture::with_tools(
        endpoint(&address)?,
        "Use your admitted file tools and summarize the exact result.",
        Arc::new(Allow),
        Arc::new(NoFaults),
        vec![TaskTool::ReadFile, TaskTool::WriteFile],
        vec![TaskPathAccess {
            path: project.path().to_string_lossy().into_owned(),
            write: true,
        }],
    )?;
    let dispatch = fixture
        .runtime
        .tick(1)?
        .pop()
        .ok_or("model task not ready")?;
    fixture.runtime.execute(dispatch)?;
    let state = fixture.state()?;
    assert_eq!(
        state.state,
        milkdrift_runtime::TaskState::Completed,
        "{state:?}"
    );
    assert_eq!(std::fs::read_to_string(&path)?, text);
    let requests = server.join().map_err(|_| "fixture server panicked")??;
    assert_eq!(requests.len(), 3);
    let last: Value = serde_json::from_str(
        requests
            .last()
            .ok_or("request absent")?
            .split_once("\r\n\r\n")
            .ok_or("request body absent")?
            .1,
    )?;
    assert!(
        last.get("messages")
            .and_then(Value::as_array)
            .ok_or("messages absent")?
            .iter()
            .any(
                |message| message.get("role").and_then(Value::as_str) == Some("tool")
                    && message.get("content").and_then(Value::as_str) == Some(text)
            )
    );
    let result = state.result.ok_or("terminal result absent")?;
    let usage = result.terminal.usage().ok_or("usage absent")?;
    assert_eq!(usage.input_units(), Some(5));
    assert_eq!(usage.output_units(), Some(7));
    assert_eq!(
        usage
            .nested_work()
            .ok_or("nested count absent")?
            .invocations()
            .model(),
        2
    );
    assert!(result.outputs.contains_key("model_response"));
    Ok(())
}

struct Revocable(Arc<AtomicBool>);
impl AuthorityEvaluator for Revocable {
    fn evaluate(
        &self,
        request: &AuthorityRequest,
    ) -> Result<AuthorityDecisionSnapshot, AuthorityError> {
        let prior = Allow.evaluate(request)?;
        if !self.0.load(Ordering::SeqCst) {
            return Ok(prior);
        }
        AuthorityDecisionSnapshot::from_evaluation(
            prior.policy().clone(),
            prior.policy_version(),
            request.clone(),
            vec![DecisionReasonCode::BudgetExcess],
            AuthorityBudget::default(),
            SideEffectClass::Unknown,
        )
    }
}

#[test]
fn authority_revoked_after_model_response_prevents_the_requested_file_effect() -> TestResult {
    let project = tempfile::tempdir()?;
    let path = project.path().join("must-not-exist.txt");
    let revoked = Arc::new(AtomicBool::new(false));
    let (address, server) = sequence(
        vec![tool("write_file", json!({"path":path,"text":"forbidden"}))],
        Some(revoked.clone()),
    )?;
    let fixture = ModelFixture::with_tools(
        endpoint(&address)?,
        "Use the file tool.",
        Arc::new(Revocable(revoked)),
        Arc::new(NoFaults),
        vec![TaskTool::WriteFile],
        vec![TaskPathAccess {
            path: project.path().to_string_lossy().into_owned(),
            write: true,
        }],
    )?;
    let dispatch = fixture
        .runtime
        .tick(1)?
        .pop()
        .ok_or("model task not ready")?;
    fixture.runtime.execute(dispatch)?;
    assert_eq!(fixture.state()?.state, milkdrift_runtime::TaskState::Failed);
    assert!(!path.exists());
    assert_eq!(
        server.join().map_err(|_| "fixture server panicked")??.len(),
        1
    );
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn ordinary_process_tool_uses_registered_profile_and_returns_exact_stdout() -> TestResult {
    let (address, server) = sequence(
        vec![
            tool("process_0", json!({"inputs":{}})),
            super::effect_stages::complete_response(),
        ],
        None,
    )?;
    let fixture = ModelFixture::with_tools(
        endpoint(&address)?,
        "Run the configured process.",
        Arc::new(Allow),
        Arc::new(NoFaults),
        vec![TaskTool::Process {
            capability: milkdrift_capability::CapabilityRequirement::new(OperationId::new(
                "process.execute",
            )?)
            .exact(CapabilityId::new("loop-process")?),
        }],
        vec![],
    )?;
    let dispatch = fixture
        .runtime
        .tick(1)?
        .pop()
        .ok_or("model task not ready")?;
    fixture.runtime.execute(dispatch)?;
    let state = fixture.state()?;
    assert_eq!(
        state.state,
        milkdrift_runtime::TaskState::Completed,
        "{state:?}"
    );
    let requests = server.join().map_err(|_| "fixture server panicked")??;
    let wire: Value = serde_json::from_str(
        requests
            .last()
            .ok_or("request absent")?
            .split_once("\r\n\r\n")
            .ok_or("body absent")?
            .1,
    )?;
    let feedback = wire
        .get("messages")
        .and_then(Value::as_array)
        .ok_or("messages absent")?
        .iter()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        .and_then(|message| message.get("content").and_then(Value::as_str))
        .ok_or("tool feedback absent")?;
    let feedback: Value = serde_json::from_str(feedback)?;
    assert_eq!(
        feedback.get("status").and_then(Value::as_str),
        Some("success")
    );
    assert_eq!(
        feedback
            .pointer("/outputs/stdout/text")
            .and_then(Value::as_str),
        Some("real process tool result\n")
    );
    assert_eq!(
        state
            .result
            .ok_or("result absent")?
            .terminal
            .usage()
            .ok_or("usage absent")?
            .nested_work()
            .ok_or("nested usage absent")?
            .invocations()
            .process(),
        1
    );
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn legacy_fixed_process_tool_claim_conflicts_before_model_entry() -> TestResult {
    use milkdrift_persistence::{NativePathClaim, NativePathClaimStore};
    let fixture = ModelFixture::with_tools(
        endpoint("127.0.0.1:9")?,
        "Run the configured process.",
        Arc::new(Allow),
        Arc::new(NoFaults),
        vec![TaskTool::Process {
            capability: milkdrift_capability::CapabilityRequirement::new(OperationId::new(
                "process.execute",
            )?)
            .exact(CapabilityId::new("loop-process")?),
        }],
        vec![],
    )?;
    let root = milkdrift_capability_host::capture_native_project_root(
        &fixture.directory.path().join("process-project"),
        true,
    )?;
    fixture.store.claim_native_paths(&NativePathClaim {
        owner: "independent-uncertain-writer".into(),
        roots: vec![root],
    })?;
    let dispatch = fixture
        .runtime
        .tick(1)?
        .pop()
        .ok_or("model task not ready")?;
    fixture.runtime.execute(dispatch)?;
    let state = fixture.state()?;
    assert_eq!(state.state, milkdrift_runtime::TaskState::Failed);
    assert!(!state.frozen.ok_or("frozen start absent")?.entered);
    assert!(
        state
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("native"))
    );
    assert!(
        fixture
            .store
            .native_path_claim("independent-uncertain-writer")?
            .is_some()
    );
    Ok(())
}

#[cfg(target_os = "linux")]
#[test]
fn configured_process_claim_never_expands_native_file_tool_access() -> TestResult {
    let allowed = tempfile::tempdir()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?.to_string();
    let fixture = ModelFixture::with_tools(
        endpoint(&address)?,
        "Read the explicitly allowed folder.",
        Arc::new(Allow),
        Arc::new(NoFaults),
        vec![
            TaskTool::ReadFile,
            TaskTool::Process {
                capability: milkdrift_capability::CapabilityRequirement::new(OperationId::new(
                    "process.execute",
                )?)
                .exact(CapabilityId::new("loop-process")?),
            },
        ],
        vec![TaskPathAccess {
            path: allowed.path().to_string_lossy().into_owned(),
            write: false,
        }],
    )?;
    let forbidden = fixture.directory.path().join("process-project/private.txt");
    std::fs::write(&forbidden, "private process bytes")?;
    let server = thread::spawn(move || -> std::io::Result<String> {
        let started = std::time::Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && started.elapsed() < Duration::from_secs(5) =>
                {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(error) => return Err(error),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let request = read_request(&mut stream)?;
        let body = tool("read_file", json!({"path":forbidden}));
        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes())?;
        Ok(request)
    });
    let dispatch = fixture
        .runtime
        .tick(1)?
        .pop()
        .ok_or("model task not ready")?;
    fixture.runtime.execute(dispatch)?;
    assert_eq!(fixture.state()?.state, milkdrift_runtime::TaskState::Failed);
    assert!(
        !server
            .join()
            .map_err(|_| "fixture server panicked")??
            .is_empty()
    );
    Ok(())
}

#[test]
fn model_call_batch_exceeding_task_ceiling_has_no_partial_file_effect() -> TestResult {
    let project = tempfile::tempdir()?;
    let path = project.path().join("must-not-exist.txt");
    let calls = (0..17).map(|index| json!({"id":format!("call_{index}"),"type":"function","function":{"name":"write_file","arguments":json!({"path":path,"text":"forbidden"}).to_string()}})).collect::<Vec<_>>();
    let body = json!({"choices":[{"message":{"content":"","tool_calls":calls},"finish_reason":"tool_calls"}],"usage":{"prompt_tokens":2,"completion_tokens":3}}).to_string();
    let (address, server) = sequence(vec![body], None)?;
    let fixture = ModelFixture::with_tools(
        endpoint(&address)?,
        "Use the bounded tool catalog.",
        Arc::new(Allow),
        Arc::new(NoFaults),
        vec![TaskTool::WriteFile],
        vec![TaskPathAccess {
            path: project.path().to_string_lossy().into_owned(),
            write: true,
        }],
    )?;
    let dispatch = fixture
        .runtime
        .tick(1)?
        .pop()
        .ok_or("model task not ready")?;
    fixture.runtime.execute(dispatch)?;
    assert_eq!(fixture.state()?.state, milkdrift_runtime::TaskState::Failed);
    assert!(!path.exists());
    assert_eq!(
        server.join().map_err(|_| "fixture server panicked")??.len(),
        1
    );
    Ok(())
}
