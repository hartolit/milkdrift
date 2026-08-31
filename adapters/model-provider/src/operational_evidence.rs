//! Deterministic, network-free entry points for operational parser evidence.

use serde::Serialize;

use crate::{anthropic, openai_compatible, stream::SseParser};

/// Aggregate work completed by both production provider stream parsers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct StreamFixtureEvidence {
    /// Complete provider responses parsed.
    pub responses: u64,
    /// Input fixture bytes consumed.
    pub input_bytes: u64,
    /// Output fragments emitted by the parsers.
    pub fragments: u64,
}

/// Exercises the production bounded SSE and provider state machines with fixed fixtures.
pub fn exercise_stream_fixtures(iterations: u32) -> Result<StreamFixtureEvidence, String> {
    let openai = concat!(
        "data: {\"id\":\"response-1\",\"model\":\"fixed\",\"choices\":[{\"delta\":{\"content\":\"milk\"},\"finish_reason\":null}]}\n\n",
        "data: {\"id\":\"response-1\",\"model\":\"fixed\",\"choices\":[{\"delta\":{\"content\":\"drift\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":2}}\n\n",
        "data: [DONE]\n\n",
    );
    let anthropic = concat!(
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"message-1\",\"model\":\"fixed\",\"usage\":{\"input_tokens\":7}}}\n\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"milkdrift\"}}\n\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    );
    let mut fragments = 0_u64;
    for _ in 0..iterations {
        parse_openai(openai.as_bytes(), &mut fragments)?;
        parse_anthropic(anthropic.as_bytes(), &mut fragments)?;
    }
    Ok(StreamFixtureEvidence {
        responses: u64::from(iterations).saturating_mul(2),
        input_bytes: u64::try_from(openai.len().saturating_add(anthropic.len()))
            .map_err(|error| error.to_string())?
            .saturating_mul(u64::from(iterations)),
        fragments,
    })
}

fn parse_openai(bytes: &[u8], fragments: &mut u64) -> Result<(), String> {
    let mut parser = SseParser::new(16_384, 65_536);
    let mut state = openai_compatible::StreamState::new();
    let mut provider_error = None;
    for chunk in bytes.chunks(17) {
        parser
            .push(chunk, |event| {
                if provider_error.is_none()
                    && let Err(error) = state.event(event, |_| {
                        *fragments = fragments.saturating_add(1);
                        Ok(())
                    })
                {
                    provider_error = Some(error.to_string());
                }
                Ok(())
            })
            .map_err(|error| error.to_string())?;
    }
    if let Some(error) = provider_error {
        return Err(error);
    }
    parser.finish().map_err(|error| error.to_string())?;
    state.complete(false).map_err(|error| error.to_string())?;
    Ok(())
}

fn parse_anthropic(bytes: &[u8], fragments: &mut u64) -> Result<(), String> {
    let mut parser = SseParser::new(16_384, 65_536);
    let mut state = anthropic::StreamState::new();
    let mut provider_error = None;
    for chunk in bytes.chunks(19) {
        parser
            .push(chunk, |event| {
                if provider_error.is_none()
                    && let Err(error) = state.event(event, |_| {
                        *fragments = fragments.saturating_add(1);
                        Ok(())
                    })
                {
                    provider_error = Some(error.to_string());
                }
                Ok(())
            })
            .map_err(|error| error.to_string())?;
    }
    if let Some(error) = provider_error {
        return Err(error);
    }
    parser.finish().map_err(|error| error.to_string())?;
    state.complete().map_err(|error| error.to_string())?;
    Ok(())
}
