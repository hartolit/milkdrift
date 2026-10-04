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
            value["protocol"],
            serde_json::to_value(ProtocolVersion::CURRENT)?
        );
        Ok((status, value))
    }

    async fn get(&self, path: &str) -> TestResult<Value> {
        let (status, reply) = self.response(Method::GET, path, None).await?;
        assert!(status.is_success(), "{path}: {reply}");
        Ok(reply["value"].clone())
    }

    async fn post(&self, path: &str, body: &Value) -> TestResult<Value> {
        let (status, reply) = self.response(Method::POST, path, Some(body)).await?;
        assert!(status.is_success(), "{path}: {reply}");
        Ok(reply["value"].clone())
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
            entry["current"] == true
                && entry["draining"] == false
                && entry["operations"]
                    .as_array()
                    .is_some_and(|ops| ops.contains(&json!("model.generate")))
        })
        .ok_or("model choice")?["capability_id"]
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
        draft = reply["value"]["draft"].clone();
        assert!(draft["mutations"].is_array());
    }
    let saved = client
        .command(
            "json-save",
            &Value::Null,
            json!({"type":"author_blueprint","draft":draft,"edit":null,"save":true}),
        )
        .await?;
    let revision = saved["value"]["revision_id"].clone();
    let revision_id = revision.as_str().ok_or("saved revision")?;
    let read = client.get(&format!("v1/revisions/{revision_id}")).await?;
    assert_eq!(read["summary"]["workflow_id"], "release-notes");
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
        authority["host"].as_str().ok_or("host")?.into(),
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
            "revision_id":revision,"inputs":[{"name":"brief","artifact_id":input["artifact_id"]}]}),
    );
    request["expected_sequence"] = json!(0);
    let mut invalid = request.clone();
    invalid["command_id"] = json!("json-missing-input");
    invalid["command"]["run_id"] = json!("invalid");
    invalid["command"]["inputs"] = json!([]);
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
        if result["run"]["terminal"].is_string() {
            break result;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("run deadline: {result}").into());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert_eq!(result["run"]["terminal"], "succeeded");
    assert_eq!(result["outputs"][0]["name"], "notes");
    let artifact = &result["outputs"][0]["artifact"];
    let artifact_id = artifact["artifact_id"].as_str().ok_or("final artifact")?;
    let metadata = client.get(&format!("v1/artifacts/{artifact_id}")).await?;
    let size = metadata["size"].as_u64().ok_or("artifact size")?;
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
        response.headers()["content-range"],
        format!("bytes 0-{}/{size}", size - 1)
    );
    let bytes = response.bytes().await?;
    assert_eq!(bytes.as_ref(), b"Harbor revised release notes");
    assert_eq!(bytes.len() as u64, size);
    assert_eq!(
        blake3::hash(&bytes).to_hex().as_str(),
        metadata["digest"].as_str().ok_or("digest")?
    );
    assert_eq!(model.requests.lock().map_err(|_| "fixture lock")?.len(), 2);

    drop(client);
    drop(daemon);
    let restarted = BinaryDaemon::start(&config, endpoint.clone(), directory.path()).await?;
    let client = JsonClient::new(endpoint)?;
    let retained: Value = serde_json::from_slice(&fs::read(&file)?)?;
    assert_eq!(client.get("v1/authority").await?, retained["authority"]);
    assert_eq!(
        client.post("v1/commands", &retained["request"]).await?["replayed"],
        true
    );
    assert_eq!(
        client.get("v1/runs/json-run/result").await?["outputs"],
        result["outputs"]
    );
    let mut changed = retained["request"].clone();
    changed["reason"] = json!("changed request");
    let (status, refusal) = client
        .response(Method::POST, "v1/commands", Some(&changed))
        .await?;
    assert_eq!(status, reqwest::StatusCode::CONFLICT);
    assert_eq!(refusal["code"], "conflict");

    let copied = client.command("json-copy", &revision,
        json!({"type":"copy_blueprint","source_revision":revision,"workflow_id":"independent-notes","name":"Independent notes"})).await?;
    let copy = copied["value"]["revision_id"]
        .as_str()
        .ok_or("copy revision")?;
    assert_ne!(copy, revision_id);
    let copy_read = client.get(&format!("v1/revisions/{copy}")).await?;
    assert_eq!(copy_read["summary"]["workflow_id"], "independent-notes");
    assert_eq!(copy_read["inputs"], read["inputs"]);
    assert_eq!(copy_read["outputs"], read["outputs"]);
    assert_eq!(
        client.get(&format!("v1/revisions/{revision_id}")).await?,
        read
    );
    assert_eq!(model.requests.lock().map_err(|_| "fixture lock")?.len(), 2);
    drop(restarted);
    Ok(())
}
