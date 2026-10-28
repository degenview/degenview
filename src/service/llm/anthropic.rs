//! Anthropic's Messages API wire format. Request building and stream decoding are pure;
//! `HttpProvider` does the I/O.

use super::sse::Event;
use super::{ChatRequest, Decoder, LlmError, Role, Step, WireRequest};
use serde_json::{Value, json};

const VERSION: &str = "2023-06-01";

pub fn build(base_url: &str, api_key: &str, request: &ChatRequest) -> WireRequest {
    let messages: Vec<Value> = request
        .messages
        .iter()
        .map(|message| {
            let role = match message.role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            json!({"role": role, "content": message.content})
        })
        .collect();
    let body = json!({
        "model": request.model,
        "max_tokens": request.max_tokens,
        "system": request.system,
        "messages": messages,
        "stream": true,
    });
    WireRequest {
        url: format!("{}/v1/messages", base_url.trim_end_matches('/')),
        headers: headers(api_key),
        body: body.to_string().into_bytes(),
    }
}

pub fn models_request(base_url: &str, api_key: &str) -> WireRequest {
    WireRequest {
        url: format!("{}/v1/models", base_url.trim_end_matches('/')),
        headers: headers(api_key),
        body: Vec::new(),
    }
}

fn headers(api_key: &str) -> Vec<(String, String)> {
    vec![
        ("content-type".to_owned(), "application/json".to_owned()),
        ("x-api-key".to_owned(), api_key.to_owned()),
        ("anthropic-version".to_owned(), VERSION.to_owned()),
    ]
}

#[derive(Default)]
pub struct AnthropicDecoder {
    truncated: bool,
    stopped: bool,
}

impl Decoder for AnthropicDecoder {
    fn feed(&mut self, event: &Event) -> Result<Step, LlmError> {
        let value: Value = serde_json::from_str(event.data.trim())
            .map_err(|_| LlmError::Protocol("unreadable stream event".to_owned()))?;
        // The `type` inside the data matches the `event:` name; read it from the data so a proxy
        // that drops the name line still works.
        match value.get("type").and_then(Value::as_str) {
            Some("content_block_delta") => {
                let text = value
                    .pointer("/delta/text")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                Ok(if text.is_empty() {
                    Step::Skip
                } else {
                    Step::Text(text.to_owned())
                })
            }
            Some("message_delta") => {
                if value.pointer("/delta/stop_reason").and_then(Value::as_str) == Some("max_tokens")
                {
                    self.truncated = true;
                }
                Ok(Step::Skip)
            }
            Some("message_stop") => {
                self.stopped = true;
                Ok(Step::End)
            }
            Some("error") => Err(LlmError::http(
                200,
                value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("the provider reported an error"),
            )),
            _ => Ok(Step::Skip), // message_start, content_block_start/stop, ping, ...
        }
    }

    fn finished(&self) -> Option<bool> {
        self.stopped.then_some(self.truncated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::llm::ChatMessage;

    fn feed(decoder: &mut AnthropicDecoder, data: &str) -> Result<Step, LlmError> {
        decoder.feed(&Event {
            name: None,
            data: data.to_owned(),
        })
    }

    #[test]
    fn the_request_carries_the_key_version_and_a_top_level_system_prompt() {
        let request = ChatRequest {
            model: "m".to_owned(),
            system: "sys".to_owned(),
            messages: vec![ChatMessage::user("hi")],
            max_tokens: 50,
        };
        let wire = build("https://api.anthropic.com/", "k", &request);
        assert_eq!(wire.url, "https://api.anthropic.com/v1/messages");
        assert!(wire.headers.contains(&("x-api-key".into(), "k".into())));
        assert!(
            wire.headers
                .contains(&("anthropic-version".into(), VERSION.into()))
        );
        let body: Value = serde_json::from_slice(&wire.body).unwrap();
        assert_eq!(body["system"], "sys");
        assert_eq!(body["max_tokens"], 50);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn text_deltas_stream_and_other_events_are_skipped() {
        let mut decoder = AnthropicDecoder::default();
        let delta =
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hi"}}"#;
        assert!(matches!(feed(&mut decoder, delta), Ok(Step::Text(text)) if text == "hi"));
        for skipped in [
            r#"{"type":"message_start","message":{}}"#,
            r#"{"type":"ping"}"#,
            r#"{"type":"content_block_stop","index":0}"#,
        ] {
            assert!(matches!(feed(&mut decoder, skipped), Ok(Step::Skip)));
        }
        assert_eq!(decoder.finished(), None);
    }

    #[test]
    fn max_tokens_marks_the_reply_truncated() {
        let mut decoder = AnthropicDecoder::default();
        feed(
            &mut decoder,
            r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"}}"#,
        )
        .unwrap();
        assert!(matches!(
            feed(&mut decoder, r#"{"type":"message_stop"}"#),
            Ok(Step::End)
        ));
        assert_eq!(decoder.finished(), Some(true));
    }

    #[test]
    fn a_clean_stop_is_not_truncated() {
        let mut decoder = AnthropicDecoder::default();
        feed(
            &mut decoder,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
        )
        .unwrap();
        feed(&mut decoder, r#"{"type":"message_stop"}"#).unwrap();
        assert_eq!(decoder.finished(), Some(false));
    }

    #[test]
    fn an_error_event_surfaces_its_message() {
        let mut decoder = AnthropicDecoder::default();
        let error =
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
        assert!(matches!(
            feed(&mut decoder, error),
            Err(LlmError::Http { message, .. }) if message == "Overloaded"
        ));
    }
}
