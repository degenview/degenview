//! A server-sent-events reader: the framing both providers stream their replies in. It knows
//! nothing about either provider's JSON, only how lines become `(event, data)` frames.

use super::LlmError;
use std::io::BufRead;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Event {
    /// The `event:` field, when the server sent one (Anthropic does; OpenAI does not).
    pub name: Option<String>,
    /// The `data:` lines of the frame, joined with `\n`.
    pub data: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Flow {
    Continue,
    Stop,
}

/// Reads frames until the stream ends, the handler says stop, or `cancel` is set. `cancel` is
/// checked between lines, so a stalled connection ends at the read timeout instead.
pub fn read_events<R: BufRead>(
    mut reader: R,
    cancel: &AtomicBool,
    mut on_event: impl FnMut(Event) -> Result<Flow, LlmError>,
) -> Result<(), LlmError> {
    let mut name: Option<String> = None;
    let mut data: Vec<String> = Vec::new();
    let mut line = Vec::new();
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(LlmError::Cancelled);
        }
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .map_err(|error| LlmError::Network(error.to_string()))?;
        let at_end = read == 0;
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches(['\r', '\n']);
        if at_end || text.is_empty() {
            // A blank line (or the end of the stream) closes the frame.
            if !data.is_empty() {
                let event = Event {
                    name: name.take(),
                    data: data.join("\n"),
                };
                data.clear();
                if on_event(event)? == Flow::Stop {
                    return Ok(());
                }
            } else {
                name = None;
            }
            if at_end {
                return Ok(());
            }
            continue;
        }
        if text.starts_with(':') {
            continue; // A comment, usually a keep-alive.
        }
        let (field, value) = match text.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (text, ""),
        };
        match field {
            "event" => name = Some(value.to_owned()),
            "data" => data.push(value.to_owned()),
            _ => {} // `id:` and `retry:` mean nothing to a one-shot request.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(input: &str) -> Vec<Event> {
        let cancel = AtomicBool::new(false);
        let mut events = Vec::new();
        read_events(input.as_bytes(), &cancel, |event| {
            events.push(event);
            Ok(Flow::Continue)
        })
        .unwrap();
        events
    }

    fn event(name: Option<&str>, data: &str) -> Event {
        Event {
            name: name.map(str::to_owned),
            data: data.to_owned(),
        }
    }

    #[test]
    fn frames_split_on_blank_lines_and_carry_the_event_name() {
        let events = collect("event: a\ndata: 1\n\ndata: 2\n\n");
        assert_eq!(events, vec![event(Some("a"), "1"), event(None, "2")]);
    }

    #[test]
    fn crlf_comments_and_multiline_data_are_handled() {
        let events = collect(": ping\r\ndata: one\r\ndata: two\r\n\r\n");
        assert_eq!(events, vec![event(None, "one\ntwo")]);
    }

    #[test]
    fn a_frame_cut_off_by_the_end_of_the_stream_still_arrives() {
        assert_eq!(collect("data: tail"), vec![event(None, "tail")]);
    }

    #[test]
    fn only_one_space_after_the_colon_is_dropped() {
        assert_eq!(collect("data:  x\n\n"), vec![event(None, " x")]);
        assert_eq!(collect("data:x\n\n"), vec![event(None, "x")]);
    }

    #[test]
    fn a_name_without_data_does_not_leak_into_the_next_frame() {
        let events = collect("event: stray\n\ndata: 1\n\n");
        assert_eq!(events, vec![event(None, "1")]);
    }

    #[test]
    fn the_handler_can_stop_early() {
        let cancel = AtomicBool::new(false);
        let mut seen = 0;
        read_events("data: 1\n\ndata: 2\n\n".as_bytes(), &cancel, |_| {
            seen += 1;
            Ok(Flow::Stop)
        })
        .unwrap();
        assert_eq!(seen, 1);
    }

    #[test]
    fn a_set_cancel_flag_ends_the_read() {
        let cancel = AtomicBool::new(true);
        let result = read_events("data: 1\n\n".as_bytes(), &cancel, |_| Ok(Flow::Continue));
        assert!(matches!(result, Err(LlmError::Cancelled)));
    }

    #[test]
    fn invalid_utf8_is_replaced_not_fatal() {
        let cancel = AtomicBool::new(false);
        let mut events = Vec::new();
        read_events(&b"data: a\xffb\n\n"[..], &cancel, |event| {
            events.push(event);
            Ok(Flow::Continue)
        })
        .unwrap();
        assert_eq!(events[0].data, "a\u{fffd}b");
    }
}
