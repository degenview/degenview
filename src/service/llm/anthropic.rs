//! Anthropic's Messages API wire format. Request building and stream decoding are pure;
//! `HttpProvider` does the I/O.

use super::sse::Event;
use super::{ChatMessage, ChatRequest, Decoder, LlmError, Role, Step, ToolCall, WireRequest};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const VERSION: &str = "2023-06-01";

pub fn build(base_url: &str, api_key: &str, request: &ChatRequest) -> WireRequest {
    let messages: Vec<Value> = request.messages.iter().map(message).collect();
    let mut body = json!({
        "model": request.model,
        "max_tokens": request.max_tokens,
        "system": request.system,
        "messages": messages,
        "stream": true,
    });
    if !request.tools.is_empty() {
        body["tools"] = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.schema,
                })
            })
            .collect();
    }
    WireRequest {
        url: format!("{}/v1/messages", base_url.trim_end_matches('/')),
        headers: headers(api_key),
        body: body.to_string().into_bytes(),
    }
}

/// One message in this format: a plain string, unless it carries tool calls or results, which are
/// content blocks. Results go first in their message, as the API requires.
fn message(message: &ChatMessage) -> Value {
    let role = match message.role {
        Role::User => "user",
        Role::Assistant => "assistant",
    };
    if message.tool_calls.is_empty() && message.tool_results.is_empty() {
        return json!({"role": role, "content": message.content});
    }
    let mut blocks: Vec<Value> = message
        .tool_results
        .iter()
        .map(|result| {
            let mut block = json!({
                "type": "tool_result",
                "tool_use_id": result.id,
                "content": result.content,
            });
            if result.is_error {
                block["is_error"] = Value::Bool(true);
            }
            block
        })
        .collect();
    if !message.content.is_empty() {
        blocks.push(json!({"type": "text", "text": message.content}));
    }
    blocks.extend(message.tool_calls.iter().map(
        |call| json!({"type": "tool_use", "id": call.id, "name": call.name, "input": call.input}),
    ));
    json!({"role": role, "content": blocks})
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

/// A tool call being assembled: its id and name arrive with the block, its arguments as
/// fragments of JSON text.
struct PendingCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
pub struct AnthropicDecoder {
    truncated: bool,
    stopped: bool,
    /// By content block index, so the calls keep the order the model made them in.
    calls: BTreeMap<u64, PendingCall>,
}

impl Decoder for AnthropicDecoder {
    fn feed(&mut self, event: &Event) -> Result<Step, LlmError> {
        let value: Value = serde_json::from_str(event.data.trim())
            .map_err(|_| LlmError::Protocol("unreadable stream event".to_owned()))?;
        // The `type` inside the data matches the `event:` name; read it from the data so a proxy
        // that drops the name line still works.
        let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
        match value.get("type").and_then(Value::as_str) {
            Some("content_block_start") => {
                // The block's own `input` is a placeholder (`{}`); the arguments come as deltas.
                if value.pointer("/content_block/type").and_then(Value::as_str) == Some("tool_use")
                {
                    let text = |pointer: &str| {
                        value
                            .pointer(pointer)
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned()
                    };
                    self.calls.insert(
                        index,
                        PendingCall {
                            id: text("/content_block/id"),
                            name: text("/content_block/name"),
                            arguments: String::new(),
                        },
                    );
                }
                Ok(Step::Skip)
            }
            Some("content_block_delta") => {
                if let Some(fragment) = value.pointer("/delta/partial_json").and_then(Value::as_str)
                {
                    if let Some(call) = self.calls.get_mut(&index) {
                        call.arguments.push_str(fragment);
                    }
                    return Ok(Step::Skip);
                }
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

    fn take_calls(&mut self) -> Vec<ToolCall> {
        std::mem::take(&mut self.calls)
            .into_values()
            .map(|call| super::make_call(call.id, call.name, &call.arguments))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::llm::{ChatMessage, ToolResult, ToolSpec};

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
            tools: Vec::new(),
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

    fn tool_request() -> ChatRequest {
        ChatRequest {
            model: "m".to_owned(),
            system: "sys".to_owned(),
            messages: vec![ChatMessage::user("find x")],
            max_tokens: 50,
            tools: vec![ToolSpec {
                name: "lookup".to_owned(),
                description: "Looks a name up.".to_owned(),
                schema: json!({"type": "object", "properties": {"name": {"type": "string"}}}),
            }],
        }
    }

    #[test]
    fn a_request_without_tools_has_no_tools_field() {
        let mut plain = tool_request();
        plain.tools.clear();
        let wire = build("https://api.anthropic.com", "k", &plain);
        let body: Value = serde_json::from_slice(&wire.body).unwrap();
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn tools_are_sent_with_an_input_schema() {
        let wire = build("https://api.anthropic.com", "k", &tool_request());
        let body: Value = serde_json::from_slice(&wire.body).unwrap();
        assert_eq!(body["tools"][0]["name"], "lookup");
        assert_eq!(body["tools"][0]["description"], "Looks a name up.");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    }

    #[test]
    fn calls_and_results_go_back_as_content_blocks() {
        let call = ToolCall {
            id: "toolu_1".to_owned(),
            name: "lookup".to_owned(),
            input: json!({"name": "x"}),
            malformed: None,
        };
        let mut request = tool_request();
        request.messages = vec![
            ChatMessage::user("find x"),
            ChatMessage::assistant_calling("Looking.", vec![call]),
            ChatMessage::tool_results(vec![
                ToolResult {
                    id: "toolu_1".to_owned(),
                    content: "found".to_owned(),
                    is_error: false,
                },
                ToolResult {
                    id: "toolu_2".to_owned(),
                    content: "no such name".to_owned(),
                    is_error: true,
                },
            ]),
        ];
        let wire = build("https://api.anthropic.com", "k", &request);
        let body: Value = serde_json::from_slice(&wire.body).unwrap();
        let assistant = &body["messages"][1];
        assert_eq!(assistant["role"], "assistant");
        assert_eq!(assistant["content"][0]["type"], "text");
        assert_eq!(assistant["content"][1]["type"], "tool_use");
        assert_eq!(assistant["content"][1]["id"], "toolu_1");
        assert_eq!(assistant["content"][1]["input"]["name"], "x");
        let results = &body["messages"][2];
        assert_eq!(results["role"], "user");
        assert_eq!(results["content"][0]["type"], "tool_result");
        assert_eq!(results["content"][0]["tool_use_id"], "toolu_1");
        assert_eq!(results["content"][0]["content"], "found");
        assert!(results["content"][0].get("is_error").is_none());
        assert_eq!(results["content"][1]["is_error"], true);
        // No empty text block beside the results.
        assert_eq!(results["content"].as_array().unwrap().len(), 2);
    }

    fn feed_all(decoder: &mut AnthropicDecoder, events: &[&str]) {
        for event in events {
            feed(decoder, event).unwrap();
        }
    }

    #[test]
    fn a_call_streamed_as_json_fragments_is_assembled_and_the_placeholder_input_ignored() {
        let mut decoder = AnthropicDecoder::default();
        feed_all(
            &mut decoder,
            &[
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Looking."}}"#,
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_1","name":"lookup","input":{}}}"#,
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":""}}"#,
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"na"}}"#,
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"me\": \"x\"}"}}"#,
                r#"{"type":"content_block_stop","index":1}"#,
                r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
                r#"{"type":"message_stop"}"#,
            ],
        );
        assert_eq!(decoder.finished(), Some(false));
        let calls = decoder.take_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "toolu_1");
        assert_eq!(calls[0].name, "lookup");
        assert_eq!(calls[0].input, json!({"name": "x"}));
    }

    #[test]
    fn a_tool_with_no_arguments_streams_nothing_and_is_an_empty_object() {
        let mut decoder = AnthropicDecoder::default();
        feed_all(
            &mut decoder,
            &[
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t","name":"compile","input":{}}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":""}}"#,
                r#"{"type":"content_block_stop","index":0}"#,
            ],
        );
        let calls = decoder.take_calls();
        assert_eq!(calls[0].input, json!({}));
        assert_eq!(calls[0].malformed, None);
    }

    #[test]
    fn two_calls_keep_their_block_order_and_bad_arguments_are_reported() {
        let mut decoder = AnthropicDecoder::default();
        feed_all(
            &mut decoder,
            &[
                r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"b","name":"second","input":{}}}"#,
                r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"a","name":"first","input":{}}}"#,
                r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"n\":"}}"#,
                r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{}"}}"#,
            ],
        );
        let calls = decoder.take_calls();
        assert_eq!((calls[0].id.as_str(), calls[1].id.as_str()), ("a", "b"));
        assert!(calls[0].malformed.is_some());
        assert_eq!(calls[1].malformed, None);
    }
}
