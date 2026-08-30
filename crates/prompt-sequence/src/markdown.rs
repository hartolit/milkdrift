use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::{PromptSequenceDocument, PromptSequenceError};

const HEADER_OPEN: &str = "```milkdrift-sequence";
const PROMPT_HEADING: &str = "## Prompt: ";

pub(crate) fn parse(bytes: &[u8]) -> Result<PromptSequenceDocument, PromptSequenceError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| PromptSequenceError::Markdown("document is not UTF-8".to_owned()))?;
    let mut lines = text.lines();
    let first = lines
        .by_ref()
        .find(|line| !line.trim().is_empty())
        .ok_or_else(|| PromptSequenceError::Markdown("document is empty".to_owned()))?;
    if first.trim() != HEADER_OPEN {
        return Err(PromptSequenceError::Markdown(format!(
            "first nonempty line must be {HEADER_OPEN}"
        )));
    }
    let mut header = String::new();
    let mut closed = false;
    for line in &mut lines {
        if line.trim() == "```" {
            closed = true;
            break;
        }
        header.push_str(line);
        header.push('\n');
    }
    if !closed || header.trim().is_empty() {
        return Err(PromptSequenceError::Markdown(
            "header fence is absent, unterminated, or empty".to_owned(),
        ));
    }
    let mut sections = BTreeMap::<String, String>::new();
    let mut current: Option<String> = None;
    let mut body = String::new();
    for line in lines {
        if let Some(identity) = line.strip_prefix(PROMPT_HEADING) {
            finish_section(&mut sections, current.take(), &mut body)?;
            let identity = identity.trim();
            if identity.is_empty() {
                return Err(PromptSequenceError::Markdown(
                    "prompt heading has no stage identity".to_owned(),
                ));
            }
            current = Some(identity.to_owned());
        } else if current.is_some() {
            body.push_str(line);
            body.push('\n');
        } else if !line.trim().is_empty() {
            return Err(PromptSequenceError::Markdown(
                "content before the first prompt heading is not allowed".to_owned(),
            ));
        }
    }
    finish_section(&mut sections, current, &mut body)?;
    if sections.is_empty() {
        return Err(PromptSequenceError::Markdown(
            "at least one '## Prompt: STAGE' section is required".to_owned(),
        ));
    }

    let mut value = milkdrift_contracts::parse_json_without_duplicates(header.as_bytes())
        .map_err(|error| PromptSequenceError::Markdown(error.to_string()))?;
    let stages = value
        .get_mut("sequence")
        .and_then(|sequence| sequence.get_mut("stages"))
        .and_then(Value::as_array_mut)
        .ok_or_else(|| {
            PromptSequenceError::Markdown("header requires sequence.stages as an array".to_owned())
        })?;
    for stage in stages {
        let object = stage.as_object_mut().ok_or_else(|| {
            PromptSequenceError::Markdown("every stage must be an object".to_owned())
        })?;
        if object.contains_key("prompt") {
            return Err(PromptSequenceError::Markdown(
                "Markdown header stages must omit prompt; sections supply exact content".to_owned(),
            ));
        }
        let identity = object.get("id").and_then(Value::as_str).ok_or_else(|| {
            PromptSequenceError::Markdown("every stage requires a string id".to_owned())
        })?;
        let content = sections.remove(identity).ok_or_else(|| {
            PromptSequenceError::Markdown(format!(
                "stage '{identity}' has no matching prompt section"
            ))
        })?;
        object.insert(
            "prompt".to_owned(),
            Value::Object(Map::from_iter([
                (
                    "type".to_owned(),
                    Value::String("inline_markdown".to_owned()),
                ),
                ("content".to_owned(), Value::String(content)),
            ])),
        );
    }
    if let Some(identity) = sections.keys().next() {
        return Err(PromptSequenceError::Markdown(format!(
            "prompt section '{identity}' has no matching stage"
        )));
    }
    PromptSequenceDocument::from_value(value)
}

fn finish_section(
    sections: &mut BTreeMap<String, String>,
    identity: Option<String>,
    body: &mut String,
) -> Result<(), PromptSequenceError> {
    let Some(identity) = identity else {
        return Ok(());
    };
    while body.ends_with('\n') {
        body.pop();
    }
    if body.trim().is_empty() {
        return Err(PromptSequenceError::Markdown(format!(
            "prompt section '{identity}' is empty"
        )));
    }
    body.push('\n');
    if sections
        .insert(identity.clone(), std::mem::take(body))
        .is_some()
    {
        return Err(PromptSequenceError::Markdown(format!(
            "duplicate prompt section '{identity}'"
        )));
    }
    Ok(())
}
