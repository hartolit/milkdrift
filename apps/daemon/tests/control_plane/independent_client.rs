//! An ordinary JSON client of a real daemon binary. Configuration/model fixtures are external;
//! every workflow choice, identity, result and recovery fact below comes from the public API.
use super::{
    binary::BinaryDaemon,
    inputs::ModelFixture,
    support::{CONTROLLER_TOKEN, TestResult},
};
use milkdrift_control_protocol::{InputUploadRequest, ProtocolVersion};
use reqwest::{Client, Method};
use serde_json::{Value, json};
use std::{fs, time::Duration};
use url::Url;

struct JsonClient {
    http: Client,
    endpoint: Url,
}
impl JsonClient {
    fn new(endpoint: Url) -> TestResult<Self> {
        Ok(Self {
            http: Client::builder().timeout(Duration::from_secs(10)).build()?,
            endpoint,
        })
    }

    async fn response(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> TestResult<(reqwest::StatusCode, Value)> {
        let mut request = self
            .http
            .request(method, self.endpoint.join(path)?)
            .bearer_auth(CONTROLLER_TOKEN);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await?;
        let status = response.status();
        let value: Value = response.json().await?;
        assert_eq!(
            value
                .pointer("/protocol")
                .ok_or("fixture field /protocol absent")?,
            &serde_json::to_value(ProtocolVersion::CURRENT)?
        );
        Ok((status, value))
    }

    async fn get(&self, path: &str) -> TestResult<Value> {
        let (status, reply) = self.response(Method::GET, path, None).await?;
        assert!(status.is_success(), "{path}: {reply}");
        Ok(reply
            .pointer("/value")
            .ok_or("fixture field /value absent")?
            .clone())
    }

    async fn post(&self, path: &str, body: &Value) -> TestResult<Value> {
        let (status, reply) = self.response(Method::POST, path, Some(body)).await?;
        assert!(status.is_success(), "{path}: {reply}");
        Ok(reply
            .pointer("/value")
            .ok_or("fixture field /value absent")?
            .clone())
    }

    async fn command(&self, id: &str, base: &Value, command: Value) -> TestResult<Value> {
        self.post("v1/commands", &envelope(id, base, command)).await
    }
}

fn envelope(id: &str, base: &Value, command: Value) -> Value {
    json!({"protocol":ProtocolVersion::CURRENT,"command_id":id,"expected_revision":base,
        "expected_sequence":null,"reason":"Independent JSON client","evidence":[],"command":command})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn json_client_authors_runs_recovers_downloads_and_copies_without_private_builders()
-> TestResult {
    let directory = tempfile::tempdir()?;
    let model = ModelFixture::start().await?;
    let (daemon, config) = BinaryDaemon::configured(&directory, &model).await?;
    let endpoint = daemon.endpoint.clone();
    let client = JsonClient::new(endpoint.clone())?;
    client
        .post("v1/version", &json!({"protocol":ProtocolVersion::CURRENT}))
        .await?;
    let authority = client.get("v1/authority").await?;
    let catalog = client.get("v1/capabilities").await?;
    let capability = catalog
        .as_array()
        .ok_or("capabilities")?
        .iter()
        .find(|entry| {
            entry.get("current").and_then(Value::as_bool) == Some(true)
                && entry.get("draining").and_then(Value::as_bool) == Some(false)
                && entry
                    .get("operations")
                    .and_then(Value::as_array)
                    .is_some_and(|ops| ops.contains(&json!("model.generate")))
        })
        .ok_or("model choice")?
        .get("capability_id")
        .ok_or("capability id absent")?
        .clone();
    assert!(capability.is_string(), "{catalog}");

    let mut draft = json!({"workflow_id":"release-notes","base_revision":null,"mutations":[]});
    let edits = [
        json!({"type":"rename","name":"Release notes"}),
        json!({"type":"input","name":"brief"}),
        json!({"type":"add_model","step":"draft","capability":capability,"prompt":"Draft release notes from the brief.","maximum_output_units":512}),
        json!({"type":"add_model","step":"review","capability":capability,"prompt":"Review the draft against the original brief.","maximum_output_units":512}),
        json!({"type":"connect","step":"draft","input":"brief","source":{"type":"run_input","name":"brief"}}),
        json!({"type":"connect","step":"review","input":"brief","source":{"type":"run_input","name":"brief"}}),
        json!({"type":"connect","step":"review","input":"draft","source":{"type":"step","step":"draft"}}),
        json!({"type":"output","step":"review","name":"notes"}),
    ];
    for (index, edit) in edits.into_iter().enumerate() {
        let reply = client
            .command(
                &format!("json-edit-{index}"),
                &Value::Null,
                json!({"type":"author_blueprint","draft":draft,"edit":edit,"save":false}),
            )
            .await?;
        draft = reply
            .pointer("/value/draft")
            .ok_or("fixture field /value/draft absent")?
            .clone();
        assert!(
            draft
                .pointer("/mutations")
                .ok_or("fixture field /mutations absent")?
                .is_array()
        );
    }
    let saved = client
        .command(
            "json-save",
            &Value::Null,
            json!({"type":"author_blueprint","draft":draft,"edit":null,"save":true}),
        )
        .await?;
    let revision = saved
        .pointer("/value/revision_id")
        .ok_or("fixture field /value/revision_id absent")?
        .clone();
    let revision_id = revision.as_str().ok_or("saved revision")?;
    let read = client.get(&format!("v1/revisions/{revision_id}")).await?;
    assert_eq!(
        read.pointer("/summary/workflow_id")
            .ok_or("fixture field /summary/workflow_id absent")?,
        "release-notes"
    );
    assert!(
        model
            .requests
            .lock()
            .map_err(|_| "fixture lock")?
            .is_empty()
    );

    // Encoding/hashing content is upload transport, not workflow identity or input selection.
    let brief = include_bytes!("../../../../examples/operator/release-notes/harbor-brief.txt");
    let upload = serde_json::to_value(InputUploadRequest::from_content(
        authority
            .pointer("/host")
            .ok_or("fixture field /host absent")?
            .as_str()
            .ok_or("host")?
            .into(),
        "json-brief".into(),
        "text/plain".into(),
        "restricted".into(),
        brief,
    )?)?;
    let input = client.post("v1/artifact-inputs", &upload).await?;
    let mut request = envelope(
        "json-start",
        &revision,
        json!({"type":"start_run","run_id":"json-run","workflow_id":"release-notes",
            "revision_id":revision,"inputs":[{"name":"brief","artifact_id":input.pointer("/artifact_id").ok_or("fixture field /artifact_id absent")?}]}),
    );
    *request
        .pointer_mut("/expected_sequence")
        .ok_or("fixture field /expected_sequence absent")? = json!(0);
    let mut invalid = request.clone();
    *invalid
        .pointer_mut("/command_id")
        .ok_or("fixture field /command_id absent")? = json!("json-missing-input");
    *invalid
        .pointer_mut("/command/run_id")
        .ok_or("fixture field /command/run_id absent")? = json!("invalid");
    *invalid
        .pointer_mut("/command/inputs")
        .ok_or("fixture field /command/inputs absent")? = json!([]);
    let (status, _) = client
        .response(Method::POST, "v1/commands", Some(&invalid))
        .await?;
    assert!(status.is_client_error());
    assert!(
        model
            .requests
            .lock()
            .map_err(|_| "fixture lock")?
            .is_empty()
    );

    let recovery = json!({"authority":authority,"request":request});
    let file = directory.path().join("json-request.json");
    fs::write(&file, serde_json::to_vec(&recovery)?)?;
    // Discard the successful response as a client that lost its reply would. It must recover
    // from the retained envelope, without needing the response's run/sequence fields.
    client.post("v1/commands", &request).await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
    let result = loop {
        let result = client.get("v1/runs/json-run/result").await?;
        if result
            .pointer("/run/terminal")
            .ok_or("fixture field /run/terminal absent")?
            .is_string()
        {
            break result;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("run deadline: {result}").into());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert_eq!(
        result
            .pointer("/run/terminal")
            .ok_or("fixture field /run/terminal absent")?,
        "succeeded"
    );
    assert_eq!(
        result
            .pointer("/outputs/0/name")
            .ok_or("fixture field /outputs/0/name absent")?,
        "notes"
    );
    let artifact = result
        .pointer("/outputs/0/artifact")
        .ok_or("fixture field /outputs/0/artifact absent")?;
    let artifact_id = artifact
        .pointer("/artifact_id")
        .ok_or("fixture field /artifact_id absent")?
        .as_str()
        .ok_or("final artifact")?;
    let metadata = client.get(&format!("v1/artifacts/{artifact_id}")).await?;
    let size = metadata
        .pointer("/size")
        .ok_or("fixture field /size absent")?
        .as_u64()
        .ok_or("artifact size")?;
    assert!(size > 0);
    let response = client
        .http
        .get(endpoint.join(&format!("v1/artifacts/{artifact_id}/content"))?)
        .bearer_auth(CONTROLLER_TOKEN)
        .header("Range", format!("bytes=0-{}", size - 1))
        .send()
        .await?;
    assert_eq!(response.status(), reqwest::StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        response
            .headers()
            .get("content-range")
            .ok_or("content range absent")?
            .to_str()?,
        format!("bytes 0-{}/{size}", size - 1)
    );
    let bytes = response.bytes().await?;
    assert_eq!(bytes.as_ref(), b"Harbor revised release notes");
    assert_eq!(bytes.len() as u64, size);
    assert_eq!(
        blake3::hash(&bytes).to_hex().as_str(),
        metadata
            .pointer("/digest")
            .ok_or("fixture field /digest absent")?
            .as_str()
            .ok_or("digest")?
    );
    assert_eq!(model.requests.lock().map_err(|_| "fixture lock")?.len(), 2);

    drop(client);
    daemon.stop()?;
    let restarted = BinaryDaemon::start(&config, endpoint.clone(), directory.path()).await?;
    let client = JsonClient::new(endpoint)?;
    let retained: Value = serde_json::from_slice(&fs::read(&file)?)?;
    assert_eq!(
        &client.get("v1/authority").await?,
        retained
            .pointer("/authority")
            .ok_or("fixture field /authority absent")?
    );
    assert_eq!(
        client
            .post(
                "v1/commands",
                retained
                    .pointer("/request")
                    .ok_or("fixture field /request absent")?
            )
            .await?
            .get("replayed")
            .ok_or("replay flag absent")?,
        true
    );
    assert_eq!(
        client
            .get("v1/runs/json-run/result")
            .await?
            .get("outputs")
            .ok_or("outputs absent")?,
        result
            .pointer("/outputs")
            .ok_or("fixture field /outputs absent")?
    );
    let mut changed = retained
        .pointer("/request")
        .ok_or("fixture field /request absent")?
        .clone();
    *changed
        .pointer_mut("/reason")
        .ok_or("fixture field /reason absent")? = json!("changed request");
    let (status, refusal) = client
        .response(Method::POST, "v1/commands", Some(&changed))
        .await?;
    assert_eq!(status, reqwest::StatusCode::CONFLICT);
    assert_eq!(
        refusal
            .pointer("/code")
            .ok_or("fixture field /code absent")?,
        "conflict"
    );

    let copied = client.command("json-copy", &revision,
        json!({"type":"copy_blueprint","source_revision":revision,"workflow_id":"independent-notes","name":"Independent notes"})).await?;
    let copy = copied
        .pointer("/value/revision_id")
        .ok_or("fixture field /value/revision_id absent")?
        .as_str()
        .ok_or("copy revision")?;
    assert_ne!(copy, revision_id);
    let copy_read = client.get(&format!("v1/revisions/{copy}")).await?;
    assert_eq!(
        copy_read
            .pointer("/summary/workflow_id")
            .ok_or("fixture field /summary/workflow_id absent")?,
        "independent-notes"
    );
    assert_eq!(
        copy_read
            .pointer("/inputs")
            .ok_or("fixture field /inputs absent")?,
        read.pointer("/inputs")
            .ok_or("fixture field /inputs absent")?
    );
    assert_eq!(
        copy_read
            .pointer("/outputs")
            .ok_or("fixture field /outputs absent")?,
        read.pointer("/outputs")
            .ok_or("fixture field /outputs absent")?
    );
    assert_eq!(
        client.get(&format!("v1/revisions/{revision_id}")).await?,
        read
    );
    assert_eq!(model.requests.lock().map_err(|_| "fixture lock")?.len(), 2);
    restarted.stop()?;
    Ok(())
}
