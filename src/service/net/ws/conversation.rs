//! One question, one answer, on a connection that stays open for the next: for a venue whose
//! history is asked over its socket (HTX), where a feed's `run` loop, which only listens, does not
//! fit.
//!
//! The session's keepalive and decoding apply as they do to a feed: the server's pings are
//! answered while an answer is awaited and never reach the caller.

use super::{Connection, Health, Session, limit_read};
use std::io;
use std::net::TcpStream;
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

/// Why a question got no answer. After any of them the connection is not asked again.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ConversationError {
    #[error("{0}")]
    Connect(String),
    #[error("the connection closed")]
    Closed,
    #[error("no answer in time")]
    NoAnswer,
}

/// An open connection that is asked one thing at a time.
pub struct Conversation {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    health: Health,
}

impl Session {
    /// Connects once and sends the opening messages, without listening for more: no reconnecting
    /// either, a conversation that breaks is begun again by whoever asked.
    pub fn converse(&self) -> Result<Conversation, ConversationError> {
        let (socket, health): Connection = self.open().map_err(ConversationError::Connect)?;
        Ok(Conversation { socket, health })
    }
}

impl Conversation {
    /// Sends `request` and waits up to `patience` for the message `answer` takes as the reply
    /// (`Some`); messages it declines (another subscription's push) are left unread.
    pub fn ask<T>(
        &mut self,
        request: &str,
        patience: Duration,
        mut answer: impl FnMut(&str) -> Option<T>,
    ) -> Result<T, ConversationError> {
        self.socket
            .send(Message::Text(request.into()))
            .map_err(|_| ConversationError::Closed)?;
        let deadline = Instant::now() + patience;
        loop {
            // A read waits as long as the time left, not the five seconds a feed's read waits, so
            // `patience` is the most an answer is waited for.
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(ConversationError::NoAnswer);
            }
            limit_read(&mut self.socket, left).map_err(|_| ConversationError::Closed)?;
            match self.health.read(&mut self.socket) {
                Ok(Some(Message::Text(text))) => {
                    if let Some(reply) = answer(text.as_str()) {
                        return Ok(reply);
                    }
                }
                Ok(Some(Message::Close(_))) => return Err(ConversationError::Closed),
                Err(tungstenite::Error::Io(error)) if error.kind() == io::ErrorKind::TimedOut => {
                    return Err(ConversationError::NoAnswer);
                }
                Err(_) => return Err(ConversationError::Closed),
                Ok(_) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Decode, Keepalive};
    use super::*;
    use std::net::TcpListener;

    fn gzip(text: &str) -> Message {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(text.as_bytes()).unwrap();
        Message::Binary(encoder.finish().unwrap().into())
    }

    fn session(port: u16, keepalive: Keepalive, decode: Decode) -> Session {
        Session {
            url: format!("ws://127.0.0.1:{port}"),
            host: "127.0.0.1".to_owned(),
            port,
            subscribe: Vec::new(),
            max_data_silence: Duration::from_secs(60),
            keepalive,
            decode,
        }
    }

    /// A server that sends `replies(message)` to each message it reads (a pong is no question) and
    /// goes on reading until the client leaves, then returns everything it read.
    fn serve(
        replies: impl Fn(&str) -> Vec<Message> + Send + 'static,
    ) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            let mut received = Vec::new();
            while let Ok(message) = socket.read() {
                let Message::Text(text) = message else {
                    continue;
                };
                received.push(text.to_string());
                if !text.as_str().starts_with(r#"{"pong""#) {
                    for reply in replies(text.as_str()) {
                        socket.send(reply).unwrap();
                    }
                }
            }
            received
        });
        (port, handle)
    }

    const HTX: Keepalive = Keepalive::EchoJson {
        ask: "ping",
        answer: "pong",
    };

    #[test]
    fn answers_come_back_on_one_connection_unpacked_and_the_servers_pings_are_echoed() {
        let (port, server) = serve(|question| {
            vec![
                gzip(r#"{"ping":1492420473027}"#),
                gzip(r#"{"id":"other","data":[]}"#),
                gzip(&format!(r#"{{"id":"{question}","data":[1]}}"#)),
            ]
        });
        let mut conversation = session(port, HTX, Decode::Gzip).converse().unwrap();
        for question in ["first", "second"] {
            let reply = conversation
                .ask(question, Duration::from_secs(5), |text| {
                    text.contains(&format!(r#""id":"{question}""#))
                        .then(|| text.to_owned())
                })
                .unwrap();
            assert_eq!(reply, format!(r#"{{"id":"{question}","data":[1]}}"#));
        }
        drop(conversation);
        let received = server.join().unwrap();
        assert_eq!(received[0], "first");
        assert!(received.contains(&r#"{"pong":1492420473027}"#.to_owned()));
        assert!(received.contains(&"second".to_owned()));
    }

    #[test]
    fn the_greeting_and_subscriptions_are_sent_before_the_first_question() {
        let (port, server) = serve(|message| {
            if message == "question" {
                vec![Message::Text("done".into())]
            } else {
                Vec::new()
            }
        });
        let mut session = session(
            port,
            Keepalive::JsonRpcHeartbeat { interval_secs: 30 },
            Decode::None,
        );
        session.subscribe = vec!["subscribe".to_owned()];
        let mut conversation = session.converse().unwrap();
        let reply = conversation
            .ask("question", Duration::from_secs(5), |text| {
                (text == "done").then(|| text.to_owned())
            })
            .unwrap();
        assert_eq!(reply, "done");
        drop(conversation);
        let received = server.join().unwrap();
        assert!(received[0].contains("public/set_heartbeat"), "{received:?}");
        assert_eq!(received[1..], ["subscribe", "question"]);
    }

    #[test]
    fn a_connection_that_cannot_be_made_is_a_connect_error() {
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let error = session(port, Keepalive::Frame, Decode::None)
            .converse()
            .err()
            .expect("nothing listens");
        assert!(matches!(error, ConversationError::Connect(_)), "{error:?}");
    }

    #[test]
    fn a_server_that_goes_away_closes_the_conversation() {
        let (port, server) = serve(|_| vec![Message::Close(None)]);
        let mut conversation = session(port, Keepalive::Frame, Decode::None)
            .converse()
            .unwrap();
        let result = conversation.ask("question", Duration::from_secs(5), |_| Some(()));
        assert_eq!(result, Err(ConversationError::Closed));
        drop(conversation);
        server.join().unwrap();
    }

    #[test]
    fn a_reply_that_never_comes_is_no_answer() {
        let (port, server) = serve(|_| vec![Message::Text("unrelated".into())]);
        let mut conversation = session(port, Keepalive::Frame, Decode::None)
            .converse()
            .unwrap();
        // The connection's read timeout is the feed's five seconds: the answer is still given up on
        // when `patience` ends, not at the first read after it.
        let started = Instant::now();
        let result = conversation.ask::<()>("question", Duration::from_millis(300), |_| None);
        assert_eq!(result, Err(ConversationError::NoAnswer));
        let waited = started.elapsed();
        assert!(waited >= Duration::from_millis(250), "{waited:?}");
        assert!(waited < Duration::from_millis(1_500), "{waited:?}");
        drop(conversation);
        server.join().unwrap();
    }
}
