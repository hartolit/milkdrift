//! Controlled model content only. Production daemon code still owns every task decision.

use milkdrift_evidence::{
    EvidenceResult,
    application::write_private,
    http_fixture::{LoopbackServer, read_request},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(super) struct Model {
    pub server: LoopbackServer,
    calls: Arc<AtomicUsize>,
}

impl Model {
    pub(super) fn start(root: &Path) -> EvidenceResult<Self> {
        let requests = root.join("controlled-model");
        fs::create_dir(&requests)?;
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let server = LoopbackServer::start(move |stream| {
            let request = read_request(stream)?;
            let index = counter.fetch_add(1, Ordering::SeqCst);
            if index >= 32 {
                return Err(std::io::Error::other(
                    "controlled model call allowance exhausted",
                ));
            }
            let (_, body) = request
                .split_once("\r\n\r\n")
                .ok_or_else(|| std::io::Error::other("model request body missing"))?;
            let body: Value = serde_json::from_str(body)?;
            let messages = body
                .get("messages")
                .and_then(Value::as_array)
                .ok_or_else(|| std::io::Error::other("model messages missing"))?;
            let user_text = messages
                .iter()
                .filter(|message| message.get("role").and_then(Value::as_str) == Some("user"))
                .filter_map(|message| message.get("content").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n");
            if user_text.is_empty() {
                return Err(std::io::Error::other(
                    "model request omitted supplied instructions and inputs",
                ));
            }
            let content = json!({
                "summary":"Controlled analysis of the supplied task brief and captured text file",
                "input_bytes":user_text.len(),"input_digest":format!("b3_{}",blake3::hash(user_text.as_bytes())),
                "brief_excerpt":milkdrift_contracts::truncate_utf8(&user_text,512),
                "detected_sections":user_text.lines().filter(|line|line.trim_start().starts_with('[')).take(16).collect::<Vec<_>>(),
                "next_checks":["Confirm the declared dependency and feature choices against the supplied brief","Compile the chosen targets before accepting an implementation"],
                "evidence_scope":"Repeatable transport and input-isolation fixture; no real-model quality claim"
            }).to_string();
            write_private(
                &requests.join(format!("request-{index}.json")),
                &serde_json::to_vec_pretty(&body)?,
            )
            .map_err(|error| std::io::Error::other(error.to_string()))?;
            write_private(
                &requests.join(format!("response-{index}.txt")),
                content.as_bytes(),
            )
            .map_err(|error| std::io::Error::other(error.to_string()))?;
            let reply = json!({"id":format!("controlled-{index}"),"model":"controlled-brief-review","choices":[{"index":0,"message":{"role":"assistant","content":content},"finish_reason":"stop"}],"usage":{"prompt_tokens":64,"completion_tokens":128,"total_tokens":192}}).to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            )?;
            Ok(())
        })?;
        Ok(Self { server, calls })
    }

    pub(super) fn profile(&self, directory: &Path) -> EvidenceResult<PathBuf> {
        let example = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/local-model/openai-compatible-loopback.example.json");
        let mut profile: Value = serde_json::from_slice(&fs::read(example)?)?;
        let object = profile
            .as_object_mut()
            .ok_or("model profile object missing")?;
        object.insert("identity".into(), json!("controlled-model-profile"));
        object.insert(
            "base_url".into(),
            json!(format!("http://{}", self.server.address())),
        );
        object.insert("model".into(), json!("controlled-brief-review"));
        object.insert("features".into(), json!(["system_role"]));
        object.insert("billing".into(),json!({"type":"unbilled","source":"bounded local controlled fixture has no provider charge"}));
        object.insert("token_limits".into(),json!({"type":"byte_bpe","template_tokens_per_message":64,"template_tokens_per_request":128,"maximum_input_tokens":65536,"maximum_output_tokens":4096,"output_control":"max_tokens","source":"bounded controlled fixture input and content response"}));
        let bytes = serde_json::to_vec_pretty(&profile)?;
        milkdrift_model_provider::EndpointProfile::from_json(&bytes)?;
        write_private(&directory.join("controlled-model-profile.json"), &bytes)
    }

    pub(super) fn finish(&mut self) -> EvidenceResult {
        self.server.finish()?;
        if self.calls.load(Ordering::SeqCst) > 32 {
            return Err("controlled model exceeded its call allowance".into());
        }
        Ok(())
    }
}
