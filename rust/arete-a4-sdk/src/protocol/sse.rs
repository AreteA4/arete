//! Incremental `text/event-stream` parser (WHATWG HTML, "Server-sent events").
//!
//! Mirror of `typescript/core/src/protocol/sse.ts`. Works on bytes so a
//! multi-byte character split across network chunks decodes correctly: line
//! terminators are ASCII, and each complete line is decoded on its own.

/// One dispatched Server-Sent Event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSentEvent {
    /// `event:` field; `message` when absent.
    pub event: String,
    pub data: String,
    /// The last event id in effect when this event was dispatched.
    pub id: Option<String>,
}

/// Feed raw chunks with [`Self::push`]; complete events come back as they finish.
#[derive(Debug, Default)]
pub struct ServerSentEventParser {
    buffer: Vec<u8>,
    event_type: String,
    data: Vec<String>,
    has_data: bool,
    last_event_id: Option<String>,
    /// Reconnection time the server asked for with `retry:`, in milliseconds.
    pub retry_ms: Option<u64>,
}

impl ServerSentEventParser {
    /// A parser resuming after `last_event_id`.
    pub fn new(last_event_id: Option<String>) -> Self {
        Self {
            last_event_id,
            ..Self::default()
        }
    }

    pub fn last_event_id(&self) -> Option<&str> {
        self.last_event_id.as_deref()
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<ServerSentEvent> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        let mut start = 0;
        let mut index = 0;
        while index < self.buffer.len() {
            match self.buffer[index] {
                b'\n' => {
                    let line = String::from_utf8_lossy(&self.buffer[start..index]).into_owned();
                    index += 1;
                    start = index;
                    events.extend(self.line(&line));
                }
                b'\r' => {
                    // A lone CR at the end of the buffer may be the first half of CRLF.
                    if index + 1 == self.buffer.len() {
                        break;
                    }
                    let line = String::from_utf8_lossy(&self.buffer[start..index]).into_owned();
                    index += if self.buffer[index + 1] == b'\n' {
                        2
                    } else {
                        1
                    };
                    start = index;
                    events.extend(self.line(&line));
                }
                _ => index += 1,
            }
        }
        self.buffer.drain(..start);
        events
    }

    /// Discard a partial event at end of stream, as the specification requires.
    pub fn end(&mut self) {
        self.buffer.clear();
        self.event_type.clear();
        self.data.clear();
        self.has_data = false;
    }

    fn line(&mut self, line: &str) -> Option<ServerSentEvent> {
        if line.is_empty() {
            return self.dispatch();
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = match line.find(':') {
            Some(colon) => {
                let value = &line[colon + 1..];
                (&line[..colon], value.strip_prefix(' ').unwrap_or(value))
            }
            None => (line, ""),
        };
        match field {
            "event" => self.event_type = value.to_string(),
            "data" => {
                self.data.push(value.to_string());
                self.has_data = true;
            }
            "id" if !value.contains('\0') => self.last_event_id = Some(value.to_string()),
            "retry" if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) => {
                self.retry_ms = value.parse().ok();
            }
            _ => {}
        }
        None
    }

    fn dispatch(&mut self) -> Option<ServerSentEvent> {
        let event = self.has_data.then(|| ServerSentEvent {
            event: if self.event_type.is_empty() {
                "message".to_string()
            } else {
                self.event_type.clone()
            },
            data: self.data.join("\n"),
            id: self.last_event_id.clone(),
        });
        self.event_type.clear();
        self.data.clear();
        self.has_data = false;
        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_events_across_chunks_and_line_endings() {
        let mut parser = ServerSentEventParser::new(None);
        let mut events = parser.push(b": comment\nevent: emit\nid: 1\nda");
        assert!(events.is_empty());
        events.extend(parser.push(b"ta: {\"a\":1}\r"));
        events.extend(parser.push(b"\n\r\nretry: 2500\ndata: x\ndata:y\r\n\r\n"));
        assert_eq!(
            events,
            vec![
                ServerSentEvent {
                    event: "emit".into(),
                    data: "{\"a\":1}".into(),
                    id: Some("1".into()),
                },
                ServerSentEvent {
                    event: "message".into(),
                    data: "x\ny".into(),
                    id: Some("1".into()),
                },
            ]
        );
        assert_eq!(parser.retry_ms, Some(2500));
    }

    #[test]
    fn decodes_characters_split_across_chunks() {
        let mut parser = ServerSentEventParser::new(Some("7".into()));
        let bytes = "data: é😀\n\n".as_bytes();
        let mut events = Vec::new();
        for byte in bytes {
            events.extend(parser.push(std::slice::from_ref(byte)));
        }
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "é😀");
        assert_eq!(events[0].id.as_deref(), Some("7"));
    }

    #[test]
    fn ignores_ids_with_nul_and_drops_partial_events_at_end() {
        let mut parser = ServerSentEventParser::new(None);
        assert!(parser.push(b"id: a\0b\nid\ndata: z").is_empty());
        assert_eq!(parser.last_event_id(), Some(""));
        parser.end();
        assert!(parser.push(b"\n").is_empty());
    }
}
