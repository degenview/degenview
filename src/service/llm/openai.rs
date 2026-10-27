//! OpenAI's chat completions wire format, which most other hosts (OpenRouter, Ollama, LM Studio,
//! Groq, ...) copy. Request building and stream decoding are pure; `HttpProvider` does the I/O.

use super::sse::Event;
use super::{ChatRequest, Decoder, LlmError, Role, Step, WireRequest};
use serde_json::{Value, json};

/// OpenAI's own reasoning models reject `max_tokens`; the copies only know it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Flavor {
    OpenAi,
    Compatible,
}

pub fn build(
    flavor: Flavor,
    base_url: &str,
    api_key: Option<&str>,
    request: &ChatRequest,
) -> WireRequest {
    let mut messages = vec![json!({"role": "system", "content": request.system})];
    messages.extend(request.messages.iter().map(|message| {
        let role = match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        json!({"role": role, "content": message.content})
    }));
    let limit_field = match flavor {
        Flavor::OpenAi => "max_completion_tokens",
        Flavor::Compatible => "max_tokens",
    };
    let body = json!({
        "model": request.model,
        "messages": messages,
        "stream": true,
        limit_field: request.max_tokens,
    });
    WireRequest {
        url: format!("{}/chat/completions", base_url.trim_end_matches('/')),
        headers: auth_headers(api_key),
        body: body.to_string().into_bytes(),
    }
}

pub fn models_request(base_url: &str, api_key: Option<&str>) -> WireRequest {
    WireRequest {
        url: format!("{}/models", base_url.trim_end_matches('/')),
        headers: auth_headers(api_key),
        body: Vec::new(),
    }
}

fn auth_headers(api_key: Option<&str>) -> Vec<(String, String)> {
    let mut headers = vec![("content-type".to_owned(), "application/json".to_owned())];
    if let Some(key) = api_key.filter(|key| !key.is_empty()) {
        headers.push(("authorization".to_owned(), format!("Bearer {key}")));
    }
    headers
}

#[derive(Default)]
pub struct OpenAiDecoder {
    finish: Option<bool>,
}

impl Decoder for OpenAiDecoder {
    fn feed(&mut self, event: &Event) -> Result<Step, LlmError> {
        let data = event.data.trim();
        if data == "[DONE]" {
            self.finish.get_or_insert(false);
            return Ok(Step::End);
        }
        let value: Value = serde_json::from_str(data)
            .map_err(|_| LlmError::Protocol("unreadable stream chunk".to_owned()))?;
        if let Some(message) = value.pointer("/error/message").and_then(Value::as_str) {
            return Err(LlmError::Http {
                status: 200,
                message: message.to_owned(),
            });
        }
        // `choices` is empty on the trailing usage chunk some hosts send.
        let Some(choice) = value.pointer("/choices/0") else {
            return Ok(Step::Skip);
        };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish = Some(reason == "length");
        }
        match choice.pointer("/delta/content").and_then(Value::as_str) {
            Some(text) if !text.is_empty() => Ok(Step::Text(text.to_owned())),
            _ => Ok(Step::Skip),
        }
    }

    fn finished(&self) -> Option<bool> {
        self.finish
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::llm::ChatMessage;

    fn request() -> ChatRequest {
        ChatRequest {
            model: "m".to_owned(),
            system: "sys".to_owned(),
            messages: vec![ChatMessage::user("hi"), ChatMessage::assistant("yo")],
            max_tokens: 100,
        }
    }

    fn feed(decoder: &mut OpenAiDecoder, data: &str) -> Result<Step, LlmError> {
        decoder.feed(&Event {
            name: None,
            data: data.to_owned(),
        })
    }

    #[test]
    fn the_request_puts_the_system_prompt_first_and_streams() {
        let wire = build(
            Flavor::OpenAi,
            "https://api.openai.com/v1/",
            Some("k"),
            &request(),
        );
        assert_eq!(wire.url, "https://api.openai.com/v1/chat/completions");
        assert!(
            wire.headers
                .contains(&("authorization".into(), "Bearer k".into()))
        );
        let body: Value = serde_json::from_slice(&wire.body).unwrap();
        assert_eq!(body["stream"], true);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["content"], "hi");
        assert_eq!(body["messages"][2]["role"], "assistant");
        assert_eq!(body["max_completion_tokens"], 100);
        assert!(body.get("max_tokens").is_none());
    }

    #[test]
    fn a_compatible_host_gets_max_tokens_and_no_header_without_a_key() {
        let wire = build(
            Flavor::Compatible,
            "http://localhost:11434/v1",
            None,
            &request(),
        );
        let body: Value = serde_json::from_slice(&wire.body).unwrap();
        assert_eq!(body["max_tokens"], 100);
        assert!(body.get("max_completion_tokens").is_none());
        assert!(wire.headers.iter().all(|(name, _)| name != "authorization"));
    }

    #[test]
    fn deltas_become_text_and_the_trailing_chunks_are_skipped() {
        let mut decoder = OpenAiDecoder::default();
        let chunk = r#"{"choices":[{"delta":{"content":"ab"},"finish_reason":null}]}"#;
        assert!(matches!(feed(&mut decoder, chunk), Ok(Step::Text(text)) if text == "ab"));
        let role = r#"{"choices":[{"delta":{"role":"assistant"}}]}"#;
        assert!(matches!(feed(&mut decoder, role), Ok(Step::Skip)));
        let usage = r#"{"choices":[],"usage":{"total_tokens":3}}"#;
        assert!(matches!(feed(&mut decoder, usage), Ok(Step::Skip)));
        assert_eq!(decoder.finished(), None);
    }

    #[test]
    fn finish_reason_length_means_truncated() {
        let mut decoder = OpenAiDecoder::default();
        feed(
            &mut decoder,
            r#"{"choices":[{"delta":{},"finish_reason":"length"}]}"#,
        )
        .unwrap();
        assert_eq!(decoder.finished(), Some(true));
        let mut decoder = OpenAiDecoder::default();
        feed(
            &mut decoder,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        )
        .unwrap();
        assert_eq!(decoder.finished(), Some(false));
    }

    #[test]
    fn done_ends_the_stream_even_without_a_finish_reason() {
        let mut decoder = OpenAiDecoder::default();
        assert!(matches!(feed(&mut decoder, "[DONE]"), Ok(Step::End)));
        assert_eq!(decoder.finished(), Some(false));
    }

    #[test]
    fn an_error_chunk_and_garbage_are_errors() {
        let mut decoder = OpenAiDecoder::default();
        let error = r#"{"error":{"message":"overloaded"}}"#;
        assert!(matches!(
            feed(&mut decoder, error),
            Err(LlmError::Http { message, .. }) if message == "overloaded"
        ));
        assert!(matches!(
            feed(&mut decoder, "<html>"),
            Err(LlmError::Protocol(_))
        ));
    }
}
