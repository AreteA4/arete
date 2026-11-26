//! Message framing for the stdio transport.
//!
//! A client writes JSON-RPC messages to stdin either one per line
//! (newline-delimited JSON) or `Content-Length` framed, as the Language Server
//! Protocol does: a block of `Name: value` header lines holding
//! `Content-Length: <bytes>`, a blank line, then exactly that many bytes of
//! JSON. The framing is detected per message: a message whose first line is a
//! header line is framed, any other line is a JSON message. Each reply uses
//! the framing of the request it answers, and a message the server starts
//! (a notification, a request) the framing last seen from the client, which
//! is newline-delimited until the first message arrives.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::ops::Range;
use std::sync::Arc;

use rmcp::model::{
    ClientJsonRpcMessage, ErrorData, JsonRpcMessage, RequestId, ServerJsonRpcMessage,
};
use rmcp::transport::Transport;
use rmcp::RoleServer;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

/// Cap on one incoming line or `Content-Length` body. A longer message is
/// dropped and logged, and the stream stays open.
const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

/// UTF-8 byte order mark, which RFC 8259 lets a JSON parser ignore.
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// How a JSON-RPC message is delimited on the stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Framing {
    /// One message per `\n`-terminated line.
    Ndjson,
    /// `Content-Length: <bytes>` (and any other headers), a blank line, the body.
    ContentLength,
}

/// MCP over a byte stream pair, in either framing; see the module docs.
pub struct StdioTransport<R, W> {
    read: BufReader<R>,
    /// Bytes read but not yet decoded.
    buf: Vec<u8>,
    /// Leading bytes of `buf` known to hold no newline, so a long first line
    /// is not rescanned from its start on every read.
    scanned: usize,
    /// Dropping the rest of an oversized line, up to its newline.
    discarding: bool,
    write: Arc<Mutex<Option<W>>>,
    /// The framing of every request still awaiting its reply. A request the
    /// client cancels is never answered, so its entry stays (a few bytes).
    requests: HashMap<RequestId, Framing>,
    last_seen: Framing,
}

impl<R, W> StdioTransport<R, W>
where
    R: AsyncRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin + 'static,
{
    pub fn new(read: R, write: W) -> Self {
        Self {
            read: BufReader::new(read),
            buf: Vec::new(),
            scanned: 0,
            discarding: false,
            write: Arc::new(Mutex::new(Some(write))),
            requests: HashMap::new(),
            last_seen: Framing::Ndjson,
        }
    }

    /// Append the next chunk of the stream to `buf`; `Ok(false)` at its end.
    ///
    /// `receive` is polled inside a `select!`, so this may be dropped at any
    /// await. `fill_buf` consumes nothing when dropped, and the copy and the
    /// `consume` after it have no await between them, so a cancelled read
    /// loses nothing.
    async fn fill(&mut self) -> io::Result<bool> {
        let taken = {
            let chunk = self.read.fill_buf().await?;
            if chunk.is_empty() {
                return Ok(false);
            }
            if self.discarding {
                if let Some(newline) = chunk.iter().position(|byte| *byte == b'\n') {
                    self.discarding = false;
                    self.buf.extend_from_slice(&chunk[newline + 1..]);
                }
            } else {
                self.buf.extend_from_slice(chunk);
            }
            chunk.len()
        };
        self.read.consume(taken);
        Ok(true)
    }
}

impl<R, W> Transport<RoleServer> for StdioTransport<R, W>
where
    R: AsyncRead + Send + Unpin,
    W: AsyncWrite + Send + Unpin + 'static,
{
    type Error = io::Error;

    fn send(
        &mut self,
        item: ServerJsonRpcMessage,
    ) -> impl Future<Output = io::Result<()>> + Send + 'static {
        let answered = match &item {
            JsonRpcMessage::Response(response) => self.requests.remove(&response.id),
            JsonRpcMessage::Error(error) => {
                error.id.as_ref().and_then(|id| self.requests.remove(id))
            }
            JsonRpcMessage::Request(_) | JsonRpcMessage::Notification(_) => None,
        };
        let frame = encode(&item, answered.unwrap_or(self.last_seen));
        let write = Arc::clone(&self.write);
        async move {
            let frame = frame?;
            let mut write = write.lock().await;
            let Some(write) = write.as_mut() else {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "Transport is closed",
                ));
            };
            write.write_all(&frame).await?;
            write.flush().await
        }
    }

    async fn receive(&mut self) -> Option<ClientJsonRpcMessage> {
        loop {
            match decode(&self.buf, self.scanned) {
                Decoded::Message { body, end, framing } => {
                    let parsed = parse(&self.buf[body]);
                    self.buf.drain(..end);
                    self.scanned = 0;
                    self.last_seen = framing;
                    match parsed {
                        Ok(message) => {
                            if let JsonRpcMessage::Request(request) = &message {
                                self.requests.insert(request.id.clone(), framing);
                            }
                            return Some(message);
                        }
                        Err(error) => match error.classify() {
                            // Not JSON: there is no id to answer, and answering
                            // could start an error storm with a peer that echoes
                            // replies back.
                            serde_json::error::Category::Syntax
                            | serde_json::error::Category::Eof => {
                                tracing::debug!("ignoring an unparsable message: {error}");
                            }
                            // JSON, but not a JSON-RPC message.
                            serde_json::error::Category::Data | serde_json::error::Category::Io => {
                                tracing::debug!(
                                    "answering a message that is not JSON-RPC: {error}"
                                );
                                let invalid = ErrorData::invalid_request("Invalid request", None);
                                tokio::spawn(self.send(ServerJsonRpcMessage::error(invalid, None)));
                            }
                        },
                    }
                }
                Decoded::Blank(len) => {
                    self.buf.drain(..len);
                    self.scanned = 0;
                }
                Decoded::Malformed { skip, reason } => {
                    tracing::warn!("dropping a malformed header block: {reason}");
                    self.buf.drain(..skip);
                    self.scanned = 0;
                }
                Decoded::Oversized => {
                    tracing::error!(
                        max_bytes = MAX_MESSAGE_BYTES,
                        "dropping an incoming message longer than the limit"
                    );
                    self.buf.clear();
                    self.scanned = 0;
                    self.discarding = true;
                }
                Decoded::Incomplete { scanned } => {
                    self.scanned = scanned;
                    match self.fill().await {
                        Ok(true) => {}
                        Ok(false) => return None,
                        Err(error) => {
                            tracing::error!("error reading the stream: {error}");
                            return None;
                        }
                    }
                }
            }
        }
    }

    async fn close(&mut self) -> io::Result<()> {
        drop(self.write.lock().await.take());
        Ok(())
    }
}

/// What the front of the buffer holds.
#[derive(Debug, PartialEq, Eq)]
enum Decoded {
    /// A whole message: its body, where its frame ends, and how it was framed.
    Message {
        body: Range<usize>,
        end: usize,
        framing: Framing,
    },
    /// Blank lines between messages.
    Blank(usize),
    /// A header block that cannot be used; drop `skip` bytes and look again.
    Malformed { skip: usize, reason: &'static str },
    /// A line longer than [`MAX_MESSAGE_BYTES`] with no end in sight.
    Oversized,
    /// Nothing complete yet; the first `scanned` bytes hold no newline.
    Incomplete { scanned: usize },
}

/// What the front of `buf` holds, given that `buf[..scanned]` has no newline.
fn decode(buf: &[u8], scanned: usize) -> Decoded {
    let blank = buf
        .iter()
        .take_while(|byte| matches!(byte, b'\r' | b'\n'))
        .count();
    if blank > 0 {
        return Decoded::Blank(blank);
    }
    let Some((text_end, next)) = line_end(buf, 0, scanned) else {
        return if buf.len() > MAX_MESSAGE_BYTES {
            Decoded::Oversized
        } else {
            Decoded::Incomplete { scanned: buf.len() }
        };
    };
    if header(&buf[..text_end]).is_some() {
        decode_headers(buf)
    } else {
        Decoded::Message {
            body: 0..text_end,
            end: next,
            framing: Framing::Ndjson,
        }
    }
}

/// A header block: `Name: value` lines up to a blank line, one of them
/// `Content-Length` with the body's size in bytes.
fn decode_headers(buf: &[u8]) -> Decoded {
    let mut at = 0;
    let mut length = None;
    loop {
        let Some((text_end, next)) = line_end(buf, at, at) else {
            return if buf.len() - at > MAX_MESSAGE_BYTES {
                Decoded::Oversized
            } else {
                Decoded::Incomplete { scanned: 0 }
            };
        };
        let line = &buf[at..text_end];
        if line.is_empty() {
            return match length {
                Some(len) if len > MAX_MESSAGE_BYTES => Decoded::Malformed {
                    skip: next,
                    reason: "Content-Length is above the message size limit",
                },
                Some(len) if buf.len() < next + len => Decoded::Incomplete { scanned: 0 },
                Some(len) => Decoded::Message {
                    body: next..next + len,
                    end: next + len,
                    framing: Framing::ContentLength,
                },
                None => Decoded::Malformed {
                    skip: next,
                    reason: "no Content-Length header",
                },
            };
        }
        let Some((name, value)) = header(line) else {
            // Drop the headers before this line and read it afresh, as a
            // message of its own.
            return Decoded::Malformed {
                skip: at,
                reason: "a line that is not a header before the blank line",
            };
        };
        if name.eq_ignore_ascii_case(b"content-length") {
            match std::str::from_utf8(value)
                .ok()
                .and_then(|value| value.trim().parse::<usize>().ok())
            {
                Some(len) => length = Some(len),
                None => {
                    return Decoded::Malformed {
                        skip: next,
                        reason: "Content-Length is not a byte count",
                    }
                }
            }
        }
        at = next;
    }
}

/// The line starting at `from`, once its `\n` has arrived: where its text
/// ends (before any `\r`) and where the next line starts. The search starts
/// at `search_from`, when the bytes before it are known to hold no newline.
fn line_end(buf: &[u8], from: usize, search_from: usize) -> Option<(usize, usize)> {
    let newline = search_from + buf[search_from..].iter().position(|byte| *byte == b'\n')?;
    let text_end = if newline > from && buf[newline - 1] == b'\r' {
        newline - 1
    } else {
        newline
    };
    Some((text_end, newline + 1))
}

/// `Name: value`, with a token for the name.
fn header(line: &[u8]) -> Option<(&[u8], &[u8])> {
    let colon = line.iter().position(|byte| *byte == b':')?;
    let (name, value) = (&line[..colon], &line[colon + 1..]);
    let token = !name.is_empty()
        && name
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    token.then_some((name, value))
}

fn parse(body: &[u8]) -> serde_json::Result<ClientJsonRpcMessage> {
    serde_json::from_slice(body.strip_prefix(UTF8_BOM).unwrap_or(body))
}

fn encode(message: &ServerJsonRpcMessage, framing: Framing) -> serde_json::Result<Vec<u8>> {
    let mut body = serde_json::to_vec(message)?;
    Ok(match framing {
        Framing::Ndjson => {
            body.push(b'\n');
            body
        }
        Framing::ContentLength => {
            let mut frame = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
            frame.extend_from_slice(&body);
            frame
        }
    })
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use rmcp::model::{JsonRpcMessage, RequestId, ServerResult};
    use rmcp::ServiceExt;
    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadBuf};

    use super::*;

    fn ping(id: i64) -> String {
        format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"ping"}}"#)
    }

    fn framed(json: &str) -> Vec<u8> {
        format!("Content-Length: {}\r\n\r\n{json}", json.len()).into_bytes()
    }

    fn line(json: &str) -> Vec<u8> {
        format!("{json}\n").into_bytes()
    }

    fn request_id(message: &ClientJsonRpcMessage) -> i64 {
        match message {
            JsonRpcMessage::Request(request) => match &request.id {
                RequestId::Number(id) => *id,
                RequestId::String(id) => panic!("string id {id}"),
            },
            other => panic!("not a request: {other:?}"),
        }
    }

    /// Every message of `input`, with the framing it arrived in.
    async fn receive_all<R: AsyncRead + Send + Unpin>(
        read: R,
    ) -> Vec<(Framing, ClientJsonRpcMessage)> {
        let mut transport = StdioTransport::new(read, Vec::new());
        let mut out = Vec::new();
        while let Some(message) = transport.receive().await {
            out.push((transport.last_seen, message));
        }
        out
    }

    async fn received_ids(input: &[u8]) -> Vec<(Framing, i64)> {
        receive_all(input)
            .await
            .iter()
            .map(|(framing, message)| (*framing, request_id(message)))
            .collect()
    }

    /// A stream that hands out one byte per read.
    struct ByteAtATime<'a>(&'a [u8]);

    impl AsyncRead for ByteAtATime<'_> {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            if let Some((first, rest)) = self.0.split_first() {
                buf.put_slice(&[*first]);
                self.0 = rest;
            }
            Poll::Ready(Ok(()))
        }
    }

    /// The replies on `client`, decoded with the framing each arrived in.
    async fn read_replies(
        client: &mut tokio::io::DuplexStream,
        count: usize,
    ) -> Vec<(Framing, Value)> {
        let mut buf = Vec::new();
        let mut replies = Vec::new();
        while replies.len() < count {
            match decode(&buf, 0) {
                Decoded::Message { body, end, framing } => {
                    replies.push((framing, serde_json::from_slice(&buf[body]).unwrap()));
                    buf.drain(..end);
                }
                Decoded::Blank(len) => {
                    buf.drain(..len);
                }
                Decoded::Incomplete { .. } => {
                    let mut chunk = [0; 4096];
                    let read = client.read(&mut chunk).await.unwrap();
                    assert!(read > 0, "stream ended after {replies:?}");
                    buf.extend_from_slice(&chunk[..read]);
                }
                other => panic!("{other:?} in replies"),
            }
        }
        replies
    }

    #[tokio::test]
    async fn ndjson_single_and_multi_message_streams() {
        let single = line(&ping(1));
        assert_eq!(received_ids(&single).await, [(Framing::Ndjson, 1)]);

        let many = [line(&ping(1)), line(&ping(2)), line(&ping(3))].concat();
        let ids = [
            (Framing::Ndjson, 1),
            (Framing::Ndjson, 2),
            (Framing::Ndjson, 3),
        ];
        assert_eq!(received_ids(&many).await, ids);
    }

    #[tokio::test]
    async fn ndjson_accepts_crlf_blank_lines_and_a_byte_order_mark() {
        let input = [
            b"\r\n\n".as_slice(),
            &format!("{}\r\n", ping(1)).into_bytes(),
            b"\n\n",
            UTF8_BOM,
            &line(&ping(2)),
        ]
        .concat();
        assert_eq!(
            received_ids(&input).await,
            [(Framing::Ndjson, 1), (Framing::Ndjson, 2)]
        );
    }

    #[tokio::test]
    async fn ndjson_drops_a_trailing_line_without_newline_and_unparsable_lines() {
        let input = [line("not json"), line(&ping(1)), ping(2).into_bytes()].concat();
        assert_eq!(received_ids(&input).await, [(Framing::Ndjson, 1)]);
    }

    #[tokio::test]
    async fn content_length_single_and_multi_message_streams() {
        let single = framed(&ping(1));
        assert_eq!(received_ids(&single).await, [(Framing::ContentLength, 1)]);

        let many = [framed(&ping(1)), framed(&ping(2)), framed(&ping(3))].concat();
        let ids = [
            (Framing::ContentLength, 1),
            (Framing::ContentLength, 2),
            (Framing::ContentLength, 3),
        ];
        assert_eq!(received_ids(&many).await, ids);
    }

    #[tokio::test]
    async fn content_length_accepts_bare_lf_extra_headers_and_any_case() {
        let json = ping(1);
        let input = format!(
            "content-type: application/vscode-jsonrpc; charset=utf-8\n\
             CONTENT-LENGTH:  {}  \n\
             X-Trace: a:b:c\n\
             \n\
             {json}",
            json.len()
        );
        assert_eq!(
            received_ids(input.as_bytes()).await,
            [(Framing::ContentLength, 1)]
        );
    }

    #[tokio::test]
    async fn content_length_counts_bytes_of_a_multibyte_body() {
        let json = r#"{"jsonrpc":"2.0","id":"héllo → 🙂","method":"ping"}"#;
        assert!(json.chars().count() < json.len());
        let input = [framed(json), framed(&ping(2))].concat();
        let messages = receive_all(input.as_slice()).await;
        let JsonRpcMessage::Request(first) = &messages[0].1 else {
            panic!("{messages:?}");
        };
        assert_eq!(first.id, RequestId::String("héllo → 🙂".into()));
        assert_eq!(request_id(&messages[1].1), 2);
    }

    #[tokio::test]
    async fn content_length_body_may_span_lines_and_be_followed_by_a_newline() {
        let json = "{\n  \"jsonrpc\": \"2.0\",\n  \"id\": 1,\n  \"method\": \"ping\"\n}";
        let input = [framed(json), b"\r\n".to_vec(), line(&ping(2))].concat();
        assert_eq!(
            received_ids(&input).await,
            [(Framing::ContentLength, 1), (Framing::Ndjson, 2)]
        );
    }

    #[tokio::test]
    async fn mixed_streams_detect_the_framing_per_message() {
        let input = [
            line(&ping(1)),
            framed(&ping(2)),
            framed(&ping(3)),
            b"\n".to_vec(),
            line(&ping(4)),
            framed(&ping(5)),
        ]
        .concat();
        assert_eq!(
            received_ids(&input).await,
            [
                (Framing::Ndjson, 1),
                (Framing::ContentLength, 2),
                (Framing::ContentLength, 3),
                (Framing::Ndjson, 4),
                (Framing::ContentLength, 5),
            ]
        );
    }

    #[tokio::test]
    async fn split_reads_one_byte_at_a_time() {
        let multibyte =
            r#"{"jsonrpc":"2.0","id":3,"method":"ping","params":{"_meta":{"note":"→🙂"}}}"#;
        let input = [
            framed(&ping(1)),
            line(&ping(2)),
            framed(multibyte),
            b"\r\n\r\n".to_vec(),
            framed(&ping(4)),
        ]
        .concat();
        let ids: Vec<_> = receive_all(ByteAtATime(&input))
            .await
            .iter()
            .map(|(framing, message)| (*framing, request_id(message)))
            .collect();
        assert_eq!(
            ids,
            [
                (Framing::ContentLength, 1),
                (Framing::Ndjson, 2),
                (Framing::ContentLength, 3),
                (Framing::ContentLength, 4),
            ]
        );
    }

    #[tokio::test]
    async fn malformed_headers_are_dropped_and_the_stream_recovers() {
        let input = [
            b"Content-Length: abc\r\n\r\n".to_vec(),
            line(&ping(1)),
            b"Content-Type: text/plain\r\n\r\n".to_vec(),
            framed(&ping(2)),
            b"Content-Length: 99999999999999\r\n\r\n".to_vec(),
            line(&ping(3)),
            b"Content-Length: 5\r\nnot a header line\r\n\r\n".to_vec(),
            line(&ping(4)),
            // A header block missing its blank line.
            b"Content-Length: 10\r\n".to_vec(),
            line(&ping(5)),
            // Cut off mid-header at the end of the stream: nothing to deliver.
            b"Content-Length: 10\r\n".to_vec(),
        ]
        .concat();
        assert_eq!(
            received_ids(&input).await,
            [
                (Framing::Ndjson, 1),
                (Framing::ContentLength, 2),
                (Framing::Ndjson, 3),
                (Framing::Ndjson, 4),
                (Framing::Ndjson, 5),
            ]
        );
    }

    #[tokio::test]
    async fn a_body_cut_off_by_the_end_of_the_stream_is_dropped() {
        let input = [
            framed(&ping(1)),
            b"Content-Length: 100\r\n\r\n{\"jsonrpc\"".to_vec(),
        ]
        .concat();
        assert_eq!(received_ids(&input).await, [(Framing::ContentLength, 1)]);
    }

    #[tokio::test]
    async fn an_oversized_line_is_dropped_and_the_next_line_delivered() {
        let huge = format!("{{\"pad\":\"{}\"}}\n", "x".repeat(MAX_MESSAGE_BYTES + 1));
        let input = [huge.into_bytes(), line(&ping(1))].concat();
        assert_eq!(received_ids(&input).await, [(Framing::Ndjson, 1)]);
    }

    #[tokio::test]
    async fn replies_mirror_the_framing_of_the_request_they_answer() {
        let input = [framed(&ping(1)), line(&ping(2)), framed(&ping(3))].concat();
        let (mut client, server) = tokio::io::duplex(64 * 1024);
        let (_, write) = tokio::io::split(server);
        let mut transport = StdioTransport::new(input.as_slice(), write);
        let before = ServerJsonRpcMessage::notification(
            rmcp::model::ServerNotification::ToolListChangedNotification(Default::default()),
        );
        transport.send(before.clone()).await.unwrap();
        for _ in 0..3 {
            transport.receive().await.unwrap();
        }
        assert!(transport.receive().await.is_none());

        // Answered out of order, each with its request's framing.
        let reply = |id: i64| {
            ServerJsonRpcMessage::response(ServerResult::empty(()), RequestId::Number(id))
        };
        transport.send(reply(2)).await.unwrap();
        transport.send(reply(1)).await.unwrap();
        let error = ServerJsonRpcMessage::error(
            ErrorData::invalid_params("bad", None),
            Some(RequestId::Number(3)),
        );
        transport.send(error).await.unwrap();
        // Not an answer: the framing last seen from the client.
        transport.send(before).await.unwrap();
        transport.close().await.unwrap();

        let mut out = Vec::new();
        client.read_to_end(&mut out).await.unwrap();
        let framed_reply = |json: &str| String::from_utf8(framed(json)).unwrap();
        let expected = [
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n".to_string(),
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{}}\n".to_string(),
            framed_reply("{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}"),
            framed_reply(
                "{\"jsonrpc\":\"2.0\",\"id\":3,\"error\":{\"code\":-32602,\"message\":\"bad\"}}",
            ),
            framed_reply("{\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}"),
        ]
        .concat();
        assert_eq!(String::from_utf8(out).unwrap(), expected);
        assert!(transport.requests.is_empty());
    }

    #[tokio::test]
    async fn json_that_is_not_a_message_is_answered_with_invalid_request() {
        let input = [framed(r#"{"hello":"world"}"#), line(&ping(1))].concat();
        let (mut client, server) = tokio::io::duplex(64 * 1024);
        let (_, write) = tokio::io::split(server);
        let mut transport = StdioTransport::new(input.as_slice(), write);
        assert_eq!(request_id(&transport.receive().await.unwrap()), 1);
        assert!(transport.receive().await.is_none());
        // The reply is written by a task of its own.
        tokio::task::yield_now().await;
        transport.close().await.unwrap();

        let mut out = Vec::new();
        client.read_to_end(&mut out).await.unwrap();
        let expected =
            framed(r#"{"jsonrpc":"2.0","error":{"code":-32600,"message":"Invalid request"}}"#);
        assert_eq!(
            String::from_utf8(out).unwrap(),
            String::from_utf8(expected).unwrap()
        );
    }

    const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"framing-test","version":"0"}}}"#;
    const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    const TOOLS_LIST: &str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;

    /// Drive the server over in-memory streams: initialize, initialized,
    /// tools/list, all in `framing`; the two replies, with their framing.
    async fn handshake_and_list_tools(framing: Framing) -> Vec<(Framing, Value)> {
        let (mut client, server) = tokio::io::duplex(1 << 20);
        let (read, write) = tokio::io::split(server);
        for json in [INITIALIZE, INITIALIZED, TOOLS_LIST] {
            let bytes = match framing {
                Framing::Ndjson => line(json),
                Framing::ContentLength => framed(json),
            };
            client.write_all(&bytes).await.unwrap();
        }
        let server = crate::AreteMcp::with_agents(
            crate::protocol_agent::ProtocolAgent::new(None),
            crate::sandbox::SandboxClient::from_values(None, None, None),
        );
        let running = server
            .serve(StdioTransport::new(read, write))
            .await
            .unwrap();
        let replies = read_replies(&mut client, 2).await;
        drop(client);
        running.waiting().await.unwrap();
        replies
    }

    fn assert_handshake_and_tools(replies: &[(Framing, Value)], framing: Framing) {
        assert_eq!(replies.len(), 2, "{replies:?}");
        assert!(
            replies.iter().all(|(seen, _)| *seen == framing),
            "{replies:?}"
        );
        assert_eq!(replies[0].1["id"], json!(1));
        assert!(
            replies[0].1["result"]["serverInfo"].is_object(),
            "{replies:?}"
        );
        assert_eq!(replies[1].1["id"], json!(2));
        let tools = replies[1].1["result"]["tools"].as_array().unwrap();
        assert!(
            tools.iter().any(|tool| tool["name"] == "explore_stacks"),
            "{tools:?}"
        );
    }

    #[tokio::test]
    async fn serves_a_newline_delimited_session() {
        let replies = handshake_and_list_tools(Framing::Ndjson).await;
        assert_handshake_and_tools(&replies, Framing::Ndjson);
    }

    #[tokio::test]
    async fn serves_a_content_length_framed_session() {
        let replies = handshake_and_list_tools(Framing::ContentLength).await;
        assert_handshake_and_tools(&replies, Framing::ContentLength);
    }
}
