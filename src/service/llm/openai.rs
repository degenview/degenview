//! OpenAI's chat completions wire format, which most other hosts (OpenRouter, Ollama, LM Studio,
//! Groq, ...) copy. Request building and stream decoding are pure; `HttpProvider` does the I/O.

use super::sse::Event;
use super::{ChatMessage, ChatRequest, Decoder, LlmError, Role, Step, ToolCall, WireRequest};
use serde_json::{Value, json};
use std::collections::BTreeMap;

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
    for message in &request.messages {
        push_message(&mut messages, message);
    }
    let limit_field = match flavor {
        Flavor::OpenAi => "max_completion_tokens",
        Flavor::Compatible => "max_tokens",
    };
    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "stream": true,
        limit_field: request.max_tokens,
    });
    if !request.tools.is_empty() {
        body["tools"] = request
            .tools
            .iter()
            .map(|tool| {
                json!({"type": "function", "function": {
                    "name": tool.name,
                    "description": tool.description,
                    "parameters": tool.schema,
                }})
            })
            .collect();
    }
    WireRequest {
        url: format!("{}/chat/completions", base_url.trim_end_matches('/')),
        headers: auth_headers(api_key),
        body: body.to_string().into_bytes(),
    }
}

/// One message in this format. A message that carries tool results is one `tool` message per
/// result, which has no error flag, so a failure says so in its text; any text beside the results
/// follows as an ordinary user message.
fn push_message(out: &mut Vec<Value>, message: &ChatMessage) {
    if !message.tool_results.is_empty() {
        for result in &message.tool_results {
            let content = if result.is_error {
                format!("Error: {}", result.content)
            } else {
                result.content.clone()
            };
            out.push(json!({"role": "tool", "tool_call_id": result.id, "content": content}));
        }
        if !message.content.is_empty() {
            out.push(json!({"role": "user", "content": message.content}));
        }
        return;
    }
    let role = match message.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    if message.tool_calls.is_empty() {
        out.push(json!({"role": role, "content": message.content}));
        return;
    }
    let calls: Vec<Value> = message
        .tool_calls
        .iter()
        .map(|call| {
            json!({
                "id": call.id,
                "type": "function",
                "function": {"name": call.name, "arguments": call.input.to_string()},
            })
        })
        .collect();
    let content = if message.content.is_empty() {
        Value::Null
    } else {
        Value::String(message.content.clone())
    };
    out.push(json!({"role": role, "content": content, "tool_calls": calls}));
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

/// A call being assembled from the fragments of its stream.
#[derive(Default)]
struct PendingCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
pub struct OpenAiDecoder {
    finish: Option<bool>,
    calls: BTreeMap<u64, PendingCall>,
}

impl OpenAiDecoder {
    /// Adds the fragments of one `tool_calls` entry. A host that sends each whole call in its own
    /// chunk, all at index 0, would otherwise glue them together: a different id at a place already
    /// taken by one starts the next call.
    fn take_fragment(&mut self, fragment: &Value) {
        let mut index = fragment.get("index").and_then(Value::as_u64).unwrap_or(0);
        let id = fragment.get("id").and_then(Value::as_str).unwrap_or("");
        if !id.is_empty()
            && self
                .calls
                .get(&index)
                .is_some_and(|call| !call.id.is_empty() && call.id != id)
        {
            index = self.calls.keys().next_back().map_or(0, |last| last + 1);
        }
        let call = self.calls.entry(index).or_default();
        if !id.is_empty() {
            call.id = id.to_owned();
        }
        if let Some(name) = fragment.pointer("/function/name").and_then(Value::as_str) {
            call.name.push_str(name);
        }
        if let Some(arguments) = fragment
            .pointer("/function/arguments")
            .and_then(Value::as_str)
        {
            call.arguments.push_str(arguments);
        }
    }
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
            return Err(LlmError::http(200, message));
        }
        // `choices` is empty on the trailing usage chunk some hosts send.
        let Some(choice) = value.pointer("/choices/0") else {
            return Ok(Step::Skip);
        };
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.finish = Some(reason == "length");
        }
        if let Some(fragments) = choice
            .pointer("/delta/tool_calls")
            .and_then(Value::as_array)
        {
            for fragment in fragments {
                self.take_fragment(fragment);
            }
        }
        match choice.pointer("/delta/content").and_then(Value::as_str) {
            Some(text) if !text.is_empty() => Ok(Step::Text(text.to_owned())),
            _ => Ok(Step::Skip),
        }
    }

    fn finished(&self) -> Option<bool> {
        self.finish
    }

    fn take_calls(&mut self) -> Vec<ToolCall> {
        std::mem::take(&mut self.calls)
            .into_values()
            .enumerate()
            .map(|(position, call)| {
                // A host that sends no id still needs one to match the result to.
                let id = if call.id.is_empty() {
                    format!("call_{position}")
                } else {
                    call.id
                };
                super::make_call(id, call.name, &call.arguments)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::llm::{ChatMessage, ToolResult, ToolSpec};

    fn request() -> ChatRequest {
        ChatRequest {
            model: "m".to_owned(),
            system: "sys".to_owned(),
            messages: vec![ChatMessage::user("hi"), ChatMessage::assistant("yo")],
            max_tokens: 100,
            tools: Vec::new(),
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

    fn lookup_tool() -> ToolSpec {
        ToolSpec {
            name: "lookup".to_owned(),
            description: "Looks a name up.".to_owned(),
            schema: json!({"type": "object", "properties": {"name": {"type": "string"}}}),
        }
    }

    #[test]
    fn a_request_without_tools_has_no_tools_field() {
        let wire = build(
            Flavor::OpenAi,
            "https://api.openai.com/v1",
            Some("k"),
            &request(),
        );
        let body: Value = serde_json::from_slice(&wire.body).unwrap();
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn tools_are_sent_as_functions() {
        let mut with_tools = request();
        with_tools.tools = vec![lookup_tool()];
        let wire = build(
            Flavor::OpenAi,
            "https://api.openai.com/v1",
            Some("k"),
            &with_tools,
        );
        let body: Value = serde_json::from_slice(&wire.body).unwrap();
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["function"]["name"], "lookup");
        assert_eq!(
            body["tools"][0]["function"]["description"],
            "Looks a name up."
        );
        assert_eq!(
            body["tools"][0]["function"]["parameters"]["properties"]["name"]["type"],
            "string"
        );
    }

    #[test]
    fn calls_and_results_go_back_as_tool_messages() {
        let call = ToolCall {
            id: "call_1".to_owned(),
            name: "lookup".to_owned(),
            input: json!({"name": "x"}),
            malformed: None,
        };
        let mut with_history = request();
        with_history.messages = vec![
            ChatMessage::user("find x"),
            ChatMessage::assistant_calling("", vec![call]),
            ChatMessage::tool_results(vec![
                ToolResult {
                    id: "call_1".to_owned(),
                    content: "found".to_owned(),
                    is_error: false,
                },
                ToolResult {
                    id: "call_2".to_owned(),
                    content: "no such name".to_owned(),
                    is_error: true,
                },
            ]),
        ];
        let wire = build(
            Flavor::OpenAi,
            "https://api.openai.com/v1",
            Some("k"),
            &with_history,
        );
        let body: Value = serde_json::from_slice(&wire.body).unwrap();
        let messages = body["messages"].as_array().unwrap();
        // system, user, assistant, one tool message per result
        assert_eq!(messages.len(), 5);
        assert_eq!(messages[2]["role"], "assistant");
        assert!(messages[2]["content"].is_null());
        assert_eq!(messages[2]["tool_calls"][0]["id"], "call_1");
        assert_eq!(messages[2]["tool_calls"][0]["function"]["name"], "lookup");
        // Arguments are a string of JSON in this format.
        assert_eq!(
            messages[2]["tool_calls"][0]["function"]["arguments"],
            r#"{"name":"x"}"#
        );
        assert_eq!(messages[3]["role"], "tool");
        assert_eq!(messages[3]["tool_call_id"], "call_1");
        assert_eq!(messages[3]["content"], "found");
        assert_eq!(messages[4]["content"], "Error: no such name");
    }

    fn feed_all(decoder: &mut OpenAiDecoder, chunks: &[&str]) {
        for chunk in chunks {
            feed(decoder, chunk).unwrap();
        }
    }

    #[test]
    fn a_call_streamed_in_fragments_is_assembled() {
        let mut decoder = OpenAiDecoder::default();
        feed_all(
            &mut decoder,
            &[
                r#"{"choices":[{"delta":{"role":"assistant","content":null,"tool_calls":[{"index":0,"id":"call_a","type":"function","function":{"name":"lookup","arguments":""}}]}}]}"#,
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"na"}}]}}]}"#,
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"me\":\"x\"}"}}]}}]}"#,
                r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            ],
        );
        assert_eq!(decoder.finished(), Some(false));
        let calls = decoder.take_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_a");
        assert_eq!(calls[0].name, "lookup");
        assert_eq!(calls[0].input, json!({"name": "x"}));
        assert_eq!(calls[0].malformed, None);
    }

    #[test]
    fn two_calls_in_one_turn_keep_their_order() {
        let mut decoder = OpenAiDecoder::default();
        feed_all(
            &mut decoder,
            &[
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"first","arguments":"{}"}},{"index":1,"id":"b","function":{"name":"second","arguments":"{\"n\":1}"}}]}}]}"#,
                r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            ],
        );
        let calls = decoder.take_calls();
        assert_eq!(
            calls.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(calls[1].input, json!({"n": 1}));
    }

    #[test]
    fn a_host_that_sends_every_call_at_index_zero_does_not_merge_them() {
        let mut decoder = OpenAiDecoder::default();
        feed_all(
            &mut decoder,
            &[
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"first","arguments":"{}"}}]}}]}"#,
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"b","function":{"name":"second","arguments":"{}"}}]}}]}"#,
            ],
        );
        let calls = decoder.take_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!((calls[0].id.as_str(), calls[1].id.as_str()), ("a", "b"));
    }

    #[test]
    fn a_tool_with_no_arguments_is_an_empty_object_and_a_missing_id_is_made_up() {
        let mut decoder = OpenAiDecoder::default();
        feed_all(
            &mut decoder,
            &[
                r#"{"choices":[{"delta":{"tool_calls":[{"function":{"name":"compile","arguments":""}}]}}]}"#,
            ],
        );
        let calls = decoder.take_calls();
        assert_eq!(calls[0].input, json!({}));
        assert_eq!(calls[0].malformed, None);
        assert_eq!(calls[0].id, "call_0");
    }

    #[test]
    fn arguments_that_are_not_json_are_reported_not_fatal() {
        let mut decoder = OpenAiDecoder::default();
        feed_all(
            &mut decoder,
            &[
                r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"lookup","arguments":"{\"name\": "}}]}}]}"#,
                r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"b","function":{"name":"lookup","arguments":"[1]"}}]}}]}"#,
            ],
        );
        let calls = decoder.take_calls();
        assert!(
            calls[0]
                .malformed
                .as_deref()
                .unwrap()
                .contains("not valid JSON")
        );
        assert!(
            calls[1]
                .malformed
                .as_deref()
                .unwrap()
                .contains("not a JSON object")
        );
        assert_eq!(calls[0].input, json!({}));
    }
}
