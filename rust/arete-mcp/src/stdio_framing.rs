//! Message framing for the stdio transport.
//!
//! The MCP stdio transport delimits JSON-RPC messages with newlines: one
//! message per line. For clients that frame messages the way the Language
//! Server Protocol does, a message may instead be `Content-Length` framed: a
//! block of `Name: value` header lines holding `Content-Length: <bytes>`, a
//! blank line, then exactly that many bytes of JSON. The framing is detected
//! per message: a message whose first line is a header line is framed, any
//! other line is a JSON message. Each reply uses the framing of the request
//! it answers, and a message the server starts (a notification, a request)
//! the framing last seen from the client, which is newline-delimited until
//! the first message arrives.
//!
//! Input that is not a message is answered with an error that has no id,
//! written before any further input is read, so a stalled stdout holds
//! stdin back: a `Parse error` for a line or framed body that is not JSON,
//! an `Invalid request` for JSON that is not a JSON-RPC message. In a framed
//! stream, bytes that are not JSON may be a frame whose `Content-Length`
//! was wrong: too long, so that the frames after it were swallowed as its
//! body, or too short, so that its tail sits in front of the next header.
//! When a `Content-Length` header line starts inside such bytes, or the
//! end of a framed body cuts the start of one short, decoding resumes
//! there and only the bytes before it are dropped, provided a usable
//! header block follows; else those bytes are dropped too.
//!
//! Each request is tracked until its reply or cancellation, for the framing
//! its reply must use, and the requests awaiting a reply are held to a
//! budget: [`MAX_OUTSTANDING_REQUESTS`] of them, with
//! [`MAX_OUTSTANDING_ID_BYTES`] of string ids between them. A request that
//! would exceed it is not passed on but answered with a `Server busy` error
//! (code -32000), with its id and in its framing, before any further input
//! is read. A request whose id is already awaiting a reply is answered the
//! same way with a `Duplicate request id` error (code -32600, an invalid
//! request): it takes no room in the budget, and the request it repeats
//! keeps its framing. The id is free again once that request is answered
//! or cancelled.
//!
//! A line longer than [`MAX_MESSAGE_BYTES`], newline included, is dropped
//! and logged, and so is a framed body longer than that: its bytes are
//! skipped as they arrive, never buffered, and the frame after it is read as
//! usual. A header block that cannot be used leaves the end of the message
//! unknown, and the bytes after it cannot be told from its body: rather than
//! guess, the transport answers with a `Parse error` and closes the stream.
//! That is a block (or a line of one) longer than [`MAX_HEADER_BYTES`], a
//! header line that starts with whitespace, a line that is not a header, or
//! a block without a usable `Content-Length`: missing, repeated, or not a
//! byte count, which is ASCII digits with at most spaces or tabs around them.

use std::collections::VecDeque;
use std::future::Future;
use std::io;
use std::ops::Range;
use std::sync::Arc;

use rmcp::model::{
    ClientJsonRpcMessage, ClientNotification, ErrorCode, ErrorData, JsonRpcMessage, RequestId,
    ServerJsonRpcMessage,
};
use rmcp::transport::Transport;
use rmcp::RoleServer;
use serde_json::error::Category;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

/// Cap on one incoming message: a line, newline included, or a framed body.
/// It is the cap `rmcp`'s own newline-delimited transport applies, so a
/// client faces the same limit on either transport.
const MAX_MESSAGE_BYTES: usize = rmcp::transport::async_rw::DEFAULT_MAX_LINE_LENGTH;

/// Cap on a header block, its blank line included, and so on each of its
/// lines. Headers carry a byte count and at most a content type, so this is
/// generous.
const MAX_HEADER_BYTES: usize = 8 * 1024;

/// Cap on requests awaiting a reply, each tracked for the framing its reply
/// must use; a request past it is answered with a `Server busy` error. It
/// is never evicted: a request is forgotten on its reply or cancellation.
const MAX_OUTSTANDING_REQUESTS: usize = 1024;

/// Cap on the string ids of the requests awaiting a reply, in bytes between
/// them, as an id may be as long as a message. Ids are short in practice:
/// this holds [`MAX_OUTSTANDING_REQUESTS`] UUIDs with room to spare.
const MAX_OUTSTANDING_ID_BYTES: usize = 64 * 1024;

/// The code of the error answering a request over the budget: one of the
/// codes JSON-RPC leaves to the server.
const SERVER_BUSY: ErrorCode = ErrorCode(-32000);

/// The name of the header that frames a message, with its colon, as
/// looked for in bytes that are not a message.
const CONTENT_LENGTH: &[u8] = b"content-length:";

/// UTF-8 byte order mark, which RFC 8259 lets a JSON parser ignore.
const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Why a header block cannot be used, as reported to the client.
const HEADERS_TOO_LONG: &str = "the header block is above the size limit";

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
    /// How far `buf` is decoded.
    cursor: Cursor,
    /// Bytes to drop from the stream before buffering again.
    skip: Option<Skip>,
    /// Bytes at the front of `buf` kept from a message that was dropped,
    /// from where the header of a frame seems to start; dropped too unless
    /// a usable header block decodes there. See `reject`.
    recovering: usize,
    write: Arc<Mutex<Writer<W>>>,
    /// A reply of the transport's own not yet written; see `write_pending`.
    pending: Option<Pending>,
    /// Whether the stream is closed at a frame whose end is unknown:
    /// `receive` reports its end once `pending` is written.
    closed: bool,
    /// The framing of every request still awaiting its reply, oldest first.
    requests: VecDeque<(RequestId, Framing)>,
    /// What the ids in `requests` count toward [`MAX_OUTSTANDING_ID_BYTES`].
    id_bytes: usize,
    last_seen: Framing,
}

/// What `fill` drops from the stream before buffering again.
#[derive(Debug, PartialEq, Eq)]
enum Skip {
    /// The rest of a line longer than the limit, through its newline.
    Line,
    /// This many bytes: the body of a framed message longer than the limit.
    Bytes(usize),
}

/// A reply the transport sends on its own behalf, until it is written.
enum Pending {
    /// Not handed to the writer yet.
    Queued(Vec<u8>),
    /// Handed to the writer, in flight there until written.
    Writing,
}

/// The output stream, shared by `send` futures and `receive`, and the frame
/// being written to it.
struct Writer<W> {
    /// `None` once closed.
    stream: Option<W>,
    /// The frame being written and how much of it is, until it is written
    /// and flushed. A write dropped part way (`receive` is polled inside a
    /// `select!`) leaves it here, and every write finishes it before
    /// starting its own frame, so frames never interleave.
    in_flight: Option<InFlight>,
}

struct InFlight {
    frame: Vec<u8>,
    written: usize,
}

impl<W: AsyncWrite + Unpin> Writer<W> {
    /// Write `frame` after the one in flight, if any, and flush.
    async fn write_frame(&mut self, frame: Vec<u8>) -> io::Result<()> {
        self.finish().await?;
        self.start(frame);
        self.finish().await
    }

    fn start(&mut self, frame: Vec<u8>) {
        debug_assert!(self.in_flight.is_none());
        self.in_flight = Some(InFlight { frame, written: 0 });
    }

    /// Write the rest of the frame in flight, if any, and flush. Each
    /// `write` either writes nothing or reports what it wrote, and `written`
    /// moves on with no await in between, so a dropped call neither repeats
    /// nor skips a byte: the next call carries on from where it stopped.
    async fn finish(&mut self) -> io::Result<()> {
        let stream = self.stream.as_mut().ok_or_else(closed)?;
        let Some(in_flight) = &mut self.in_flight else {
            return Ok(());
        };
        while in_flight.written < in_flight.frame.len() {
            match stream.write(&in_flight.frame[in_flight.written..]).await? {
                0 => return Err(io::ErrorKind::WriteZero.into()),
                written => in_flight.written += written,
            }
        }
        stream.flush().await?;
        self.in_flight = None;
        Ok(())
    }
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
            cursor: Cursor::START,
            skip: None,
            recovering: 0,
            write: Arc::new(Mutex::new(Writer {
                stream: Some(write),
                in_flight: None,
            })),
            pending: None,
            closed: false,
            requests: VecDeque::new(),
            id_bytes: 0,
            last_seen: Framing::Ndjson,
        }
    }

    /// Append the next chunk of the stream to `buf`, less what `skip` drops;
    /// `Ok(false)` at its end.
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
            // Where buffering resumes in this chunk, if the skip ends in it.
            let resume = match &mut self.skip {
                None => Some(0),
                Some(Skip::Line) => chunk
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map(|newline| newline + 1),
                Some(Skip::Bytes(left)) if *left <= chunk.len() => Some(*left),
                Some(Skip::Bytes(left)) => {
                    *left -= chunk.len();
                    None
                }
            };
            if let Some(from) = resume {
                self.skip = None;
                self.buf.extend_from_slice(&chunk[from..]);
            }
            chunk.len()
        };
        self.read.consume(taken);
        Ok(true)
    }

    /// Write the pending reply, after any frame in flight, and flush. The
    /// reply moves to the writer in the poll that marks it `Writing`, so a
    /// dropped call knows to wait for it rather than queue it again.
    async fn write_pending(&mut self) -> io::Result<()> {
        if self.pending.is_none() {
            return Ok(());
        }
        let mut writer = self.write.lock().await;
        writer.finish().await?;
        if let Some(Pending::Queued(frame)) = self.pending.replace(Pending::Writing) {
            writer.start(frame);
        }
        writer.finish().await?;
        self.pending = None;
        Ok(())
    }

    /// Queue an error reply: with no id, for input that is not a message, or
    /// with the id of a request the transport answers itself. The one before
    /// it was written before that input was decoded, so there is never more
    /// than one.
    fn fail(&mut self, framing: Framing, error: ErrorData, id: Option<RequestId>) {
        debug_assert!(self.pending.is_none());
        match encode(&ServerJsonRpcMessage::error(error, id), framing) {
            Ok(frame) => self.pending = Some(Pending::Queued(frame)),
            Err(error) => tracing::error!("cannot encode an error reply: {error}"),
        }
    }

    /// Drop a message that does not parse, answering it with an error; see
    /// the module docs.
    fn reject(
        &mut self,
        body: Range<usize>,
        end: usize,
        framing: Framing,
        error: serde_json::Error,
    ) {
        let (framing, drop, error) = match error.classify() {
            // Not JSON. A framed stream stays framed through it, and the bytes
            // may hold the header of the frame after one whose declared length
            // was wrong, whole or cut short by their end: decoding resumes at
            // that header, and the bytes kept from there are dropped too unless
            // a usable header block decodes; see `recovering`.
            Category::Syntax | Category::Eof => {
                tracing::debug!("dropping input that is not JSON: {error}");
                let framing = if self.last_seen == Framing::ContentLength {
                    Framing::ContentLength
                } else {
                    framing
                };
                let resume = match framing {
                    Framing::ContentLength => header_start(&self.buf[body.clone()]),
                    Framing::Ndjson => None,
                };
                let drop = resume.map_or(end, |at| body.start + at);
                (framing, drop, parse_error(error.to_string()))
            }
            // JSON, but not a JSON-RPC message.
            Category::Data | Category::Io => {
                tracing::debug!("answering a message that is not JSON-RPC: {error}");
                let invalid = ErrorData::invalid_request("Invalid request", None);
                (framing, end, invalid)
            }
        };
        self.fail(framing, error, None);
        self.buf.drain(..drop);
        self.recovering = end - drop;
        self.last_seen = framing;
    }

    /// Remember the framing of a request, and forget a cancelled one: it is
    /// never answered. `false` for a request that is not passed on but
    /// answered here: one over the budget, or one whose id is already
    /// awaiting a reply. JSON-RPC 2.0 wants an id unique among those, so
    /// the repeat is an invalid request (code -32600) rather than a busy
    /// server's; it takes no room in the budget, and the request it repeats
    /// keeps its framing.
    fn track(&mut self, message: &ClientJsonRpcMessage, framing: Framing) -> bool {
        match message {
            JsonRpcMessage::Request(request) => {
                if self.requests.iter().any(|(id, _)| *id == request.id) {
                    tracing::debug!(
                        "answering a request whose id is already awaiting a reply with an error"
                    );
                    let duplicate = ErrorData::invalid_request("Duplicate request id", None);
                    self.fail(framing, duplicate, Some(request.id.clone()));
                    return false;
                }
                let bytes = id_bytes(&request.id);
                let over = if self.requests.len() == MAX_OUTSTANDING_REQUESTS {
                    Some("too many requests awaiting a reply")
                } else if self.id_bytes + bytes > MAX_OUTSTANDING_ID_BYTES {
                    Some("the ids of the requests awaiting a reply are above the size limit")
                } else {
                    None
                };
                if let Some(reason) = over {
                    tracing::warn!(
                        outstanding = self.requests.len(),
                        id_bytes = self.id_bytes,
                        "answering a request over the budget with an error: {reason}"
                    );
                    let busy = ErrorData::new(SERVER_BUSY, "Server busy", Some(reason.into()));
                    self.fail(framing, busy, Some(request.id.clone()));
                    return false;
                }
                self.id_bytes += bytes;
                self.requests.push_back((request.id.clone(), framing));
            }
            JsonRpcMessage::Notification(notification) => {
                if let ClientNotification::CancelledNotification(cancelled) =
                    &notification.notification
                {
                    if let Some(id) = &cancelled.params.request_id {
                        self.untrack(id);
                    }
                }
            }
            JsonRpcMessage::Response(_) | JsonRpcMessage::Error(_) => {}
        }
        true
    }

    /// The framing of the request `id`, forgetting it.
    fn untrack(&mut self, id: &RequestId) -> Option<Framing> {
        let at = self
            .requests
            .iter()
            .position(|(tracked, _)| tracked == id)?;
        self.id_bytes -= id_bytes(id);
        self.requests.remove(at).map(|(_, framing)| framing)
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
            JsonRpcMessage::Response(response) => self.untrack(&response.id),
            JsonRpcMessage::Error(error) => error.id.as_ref().and_then(|id| self.untrack(id)),
            JsonRpcMessage::Request(_) | JsonRpcMessage::Notification(_) => None,
        };
        let frame = encode(&item, answered.unwrap_or(self.last_seen));
        let write = Arc::clone(&self.write);
        async move {
            let frame = frame?;
            write.lock().await.write_frame(frame).await
        }
    }

    async fn receive(&mut self) -> Option<ClientJsonRpcMessage> {
        loop {
            if let Err(error) = self.write_pending().await {
                tracing::error!("error writing a reply: {error}");
                return None;
            }
            if self.closed {
                return None;
            }
            let decoded = decode(&self.buf, self.cursor);
            if self.recovering > 0 {
                match decoded {
                    Decoded::Incomplete(_) => {}
                    Decoded::Message {
                        framing: Framing::ContentLength,
                        ..
                    }
                    | Decoded::OversizedBody { .. } => self.recovering = 0,
                    _ => {
                        // No usable header block: the bytes kept belonged to
                        // the dropped message after all.
                        tracing::debug!("dropping input kept as a possible header");
                        self.buf.drain(..self.recovering);
                        self.recovering = 0;
                        self.cursor = Cursor::START;
                        continue;
                    }
                }
            }
            match decoded {
                Decoded::Message { body, end, framing } => {
                    self.cursor = Cursor::START;
                    match parse(&self.buf[body.clone()]) {
                        Ok(message) => {
                            self.buf.drain(..end);
                            self.last_seen = framing;
                            if self.track(&message, framing) {
                                return Some(message);
                            }
                        }
                        Err(error) => self.reject(body, end, framing, error),
                    }
                }
                Decoded::Blank(len) => {
                    self.buf.drain(..len);
                    self.cursor = Cursor::START;
                }
                Decoded::OversizedLine { end } => {
                    tracing::error!(
                        max_bytes = MAX_MESSAGE_BYTES,
                        "dropping a line longer than the limit"
                    );
                    match end {
                        Some(end) => {
                            self.buf.drain(..end);
                        }
                        None => {
                            self.buf.clear();
                            self.skip = Some(Skip::Line);
                        }
                    }
                    self.cursor = Cursor::START;
                }
                Decoded::OversizedBody { start, len } => {
                    tracing::error!(
                        max_bytes = MAX_MESSAGE_BYTES,
                        "skipping a framed body longer than the limit"
                    );
                    let buffered = len.min(self.buf.len() - start);
                    self.buf.drain(..start + buffered);
                    if buffered < len {
                        self.skip = Some(Skip::Bytes(len - buffered));
                    }
                    self.cursor = Cursor::START;
                }
                Decoded::Malformed(reason) => {
                    tracing::error!(
                        "closing the stream at a header block that cannot be used: {reason}"
                    );
                    self.fail(Framing::ContentLength, parse_error(reason), None);
                    self.closed = true;
                }
                Decoded::Incomplete(cursor) => {
                    self.cursor = cursor;
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
        drop(self.write.lock().await.stream.take());
        Ok(())
    }
}

fn closed() -> io::Error {
    io::Error::new(io::ErrorKind::NotConnected, "Transport is closed")
}

/// A `Parse error`, with why as its `data`.
fn parse_error(reason: impl Into<String>) -> ErrorData {
    ErrorData::parse_error(
        "Parse error",
        Some(serde_json::Value::String(reason.into())),
    )
}

/// Where decoding stands in `buf`, kept across reads so that a message
/// arriving in pieces is not rescanned from its start on every read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cursor {
    /// At the start of a message; the first `scanned` bytes hold no newline.
    Line { scanned: usize },
    /// In a header block: the next line starts at `at`, the bytes from there
    /// to `scanned` hold no newline, and the `Content-Length` seen so far.
    Headers {
        at: usize,
        scanned: usize,
        length: Option<usize>,
    },
    /// After a header block: `len` bytes of body from `start`.
    Body { start: usize, len: usize },
}

impl Cursor {
    const START: Self = Cursor::Line { scanned: 0 };
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
    /// A line longer than [`MAX_MESSAGE_BYTES`], ending at `end` once its
    /// newline has arrived.
    OversizedLine { end: Option<usize> },
    /// A framed body longer than [`MAX_MESSAGE_BYTES`]: `len` bytes from `start`.
    OversizedBody { start: usize, len: usize },
    /// A header block that cannot be used, so where the message ends is unknown.
    Malformed(&'static str),
    /// Nothing complete yet; decoding resumes from the cursor.
    Incomplete(Cursor),
}

/// What the front of `buf` holds, decoding on from `cursor`.
fn decode(buf: &[u8], cursor: Cursor) -> Decoded {
    let (mut at, mut scanned, mut length) = match cursor {
        Cursor::Line { scanned } => {
            let blank = blank_lines(buf);
            if blank > 0 {
                return Decoded::Blank(blank);
            }
            let Some((text_end, next)) = line_end(buf, 0, scanned) else {
                // A header line is told from its first bytes, and is held to
                // the header cap rather than the line cap: checked once, on
                // the read that takes the line past it.
                if scanned <= MAX_HEADER_BYTES
                    && buf.len() > MAX_HEADER_BYTES
                    && header_like(&buf[..MAX_HEADER_BYTES])
                {
                    return Decoded::Malformed(HEADERS_TOO_LONG);
                }
                return if buf.len() > MAX_MESSAGE_BYTES {
                    Decoded::OversizedLine { end: None }
                } else {
                    Decoded::Incomplete(Cursor::Line { scanned: buf.len() })
                };
            };
            if next > MAX_MESSAGE_BYTES {
                return Decoded::OversizedLine { end: Some(next) };
            }
            let line = &buf[..text_end];
            if header(line).is_none() {
                return if header_like(line) {
                    Decoded::Malformed("a header line that starts with whitespace")
                } else {
                    Decoded::Message {
                        body: 0..text_end,
                        end: next,
                        framing: Framing::Ndjson,
                    }
                };
            }
            // A header block: walk it from this line, whose newline is known.
            (0, next - 1, None)
        }
        Cursor::Headers {
            at,
            scanned,
            length,
        } => (at, scanned, length),
        Cursor::Body { start, len } => return framed_body(buf, start, len),
    };
    loop {
        let Some((text_end, next)) = line_end(buf, at, scanned) else {
            return if buf.len() > MAX_HEADER_BYTES {
                Decoded::Malformed(HEADERS_TOO_LONG)
            } else {
                Decoded::Incomplete(Cursor::Headers {
                    at,
                    scanned: buf.len(),
                    length,
                })
            };
        };
        if next > MAX_HEADER_BYTES {
            return Decoded::Malformed(HEADERS_TOO_LONG);
        }
        let line = &buf[at..text_end];
        if line.is_empty() {
            return match length {
                Some(len) => framed_body(buf, next, len),
                None => Decoded::Malformed("no Content-Length header"),
            };
        }
        let Some((name, value)) = header(line) else {
            return Decoded::Malformed("a line that is not a header before the blank line");
        };
        if name.eq_ignore_ascii_case(b"content-length") {
            if length.is_some() {
                return Decoded::Malformed("more than one Content-Length header");
            }
            length = byte_count(value);
            if length.is_none() {
                return Decoded::Malformed("Content-Length is not a byte count");
            }
        }
        at = next;
        scanned = next;
    }
}

/// The body of `len` bytes from `start`, once all of it has arrived.
fn framed_body(buf: &[u8], start: usize, len: usize) -> Decoded {
    if len > MAX_MESSAGE_BYTES {
        Decoded::OversizedBody { start, len }
    } else if buf.len() < start + len {
        Decoded::Incomplete(Cursor::Body { start, len })
    } else {
        Decoded::Message {
            body: start..start + len,
            end: start + len,
            framing: Framing::ContentLength,
        }
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
    let token = !name.is_empty() && name.iter().all(|byte| token_byte(*byte));
    token.then_some((name, value))
}

/// Whether `line` is a header line once leading whitespace is ignored.
fn header_like(line: &[u8]) -> bool {
    header(line.trim_ascii_start()).is_some()
}

fn token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}

/// A `Content-Length` value: ASCII digits, with spaces or tabs around them
/// at most.
fn byte_count(value: &[u8]) -> Option<usize> {
    let digits = std::str::from_utf8(value)
        .ok()?
        .trim_matches(|byte| byte == ' ' || byte == '\t');
    let numeric = !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit());
    numeric.then(|| digits.parse().ok()).flatten()
}

/// The empty lines at the start of `buf`: each a `\n` or a `\r\n`. Any other
/// byte starts a line, a `\r` on its own included.
fn blank_lines(buf: &[u8]) -> usize {
    let mut at = 0;
    loop {
        at += match buf[at..] {
            [b'\n', ..] => 1,
            [b'\r', b'\n', ..] => 2,
            _ => return at,
        };
    }
}

/// Where a `Content-Length` header line starts in `bytes`, if one does, or
/// where the start of one is cut short by their end: at their start, or
/// after a byte that no header name holds.
fn header_start(bytes: &[u8]) -> Option<usize> {
    (0..bytes.len()).find(|&at| {
        let starts = at == 0 || !token_byte(bytes[at - 1]);
        let len = CONTENT_LENGTH.len().min(bytes.len() - at);
        starts && bytes[at..at + len].eq_ignore_ascii_case(&CONTENT_LENGTH[..len])
    })
}

/// What an id counts toward [`MAX_OUTSTANDING_ID_BYTES`]: the bytes of a
/// string; a number takes no room beyond its entry.
fn id_bytes(id: &RequestId) -> usize {
    match id {
        RequestId::String(id) => id.len(),
        RequestId::Number(_) => 0,
    }
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
    use std::time::{Duration, Instant};

    use rmcp::model::{JsonRpcMessage, RequestId, ServerResult};
    use rmcp::ServiceExt;
    use serde_json::{json, Value};
    use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadBuf};

    use super::*;

    const PARSE_ERROR: i64 = -32700;
    const INVALID_REQUEST: i64 = -32600;

    fn ping(id: i64) -> String {
        format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"ping"}}"#)
    }

    fn framed(json: &str) -> Vec<u8> {
        format!("Content-Length: {}\r\n\r\n{json}", json.len()).into_bytes()
    }

    /// `json` framed with a `Content-Length` of `declared`, right or not.
    fn framed_as(json: &str, declared: usize) -> Vec<u8> {
        format!("Content-Length: {declared}\r\n\r\n{json}").into_bytes()
    }

    fn line(json: &str) -> Vec<u8> {
        format!("{json}\n").into_bytes()
    }

    /// A ping whose line, newline included, is `size` bytes.
    fn sized_line(id: i64, size: usize) -> Vec<u8> {
        let head =
            format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"ping","params":{{"_meta":{{"pad":""#);
        let tail = "\"}}}\n";
        let pad = "x".repeat(size - head.len() - tail.len());
        format!("{head}{pad}{tail}").into_bytes()
    }

    fn cancel(id: i64) -> String {
        format!(
            r#"{{"jsonrpc":"2.0","method":"notifications/cancelled","params":{{"requestId":{id}}}}}"#
        )
    }

    fn reply(id: i64) -> ServerJsonRpcMessage {
        ServerJsonRpcMessage::response(ServerResult::empty(()), RequestId::Number(id))
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

    /// Drive the transport over `read` to the end of the stream: every
    /// message with the framing it arrived in, and what the transport wrote
    /// on its own.
    async fn drive<R: AsyncRead + Send + Unpin>(
        read: R,
    ) -> (Vec<(Framing, ClientJsonRpcMessage)>, Vec<u8>) {
        let mut transport = StdioTransport::new(read, Vec::new());
        let mut messages = Vec::new();
        while let Some(message) = transport.receive().await {
            messages.push((transport.last_seen, message));
        }
        let written = transport.write.lock().await.stream.take().unwrap();
        (messages, written)
    }

    /// The id of every request of `read`, with the framing it arrived in,
    /// and the error replies written: the framing, code and reason of each.
    async fn ids_and_errors<R: AsyncRead + Send + Unpin>(
        read: R,
    ) -> (Vec<(Framing, i64)>, Vec<(Framing, i64, String)>) {
        let (messages, written) = drive(read).await;
        let ids = messages
            .iter()
            .map(|(framing, message)| (*framing, request_id(message)))
            .collect();
        (ids, errors_in(&written))
    }

    async fn ids_from<R: AsyncRead + Send + Unpin>(read: R) -> Vec<(Framing, i64)> {
        ids_and_errors(read).await.0
    }

    async fn received_ids(input: &[u8]) -> Vec<(Framing, i64)> {
        ids_from(input).await
    }

    /// The messages in `bytes`, each with the framing it is in.
    fn messages_in(mut bytes: &[u8]) -> Vec<(Framing, Value)> {
        let mut messages = Vec::new();
        loop {
            match decode(bytes, Cursor::START) {
                Decoded::Message { body, end, framing } => {
                    messages.push((framing, serde_json::from_slice(&bytes[body]).unwrap()));
                    bytes = &bytes[end..];
                }
                Decoded::Blank(len) => bytes = &bytes[len..],
                Decoded::Incomplete(_) => {
                    assert!(bytes.is_empty(), "{} undecoded bytes", bytes.len());
                    return messages;
                }
                other => panic!("{other:?} in the output"),
            }
        }
    }

    /// The error replies that make up `bytes`: the framing, code and reason
    /// (the `data` of a parse error, else the message) of each.
    fn errors_in(bytes: &[u8]) -> Vec<(Framing, i64, String)> {
        messages_in(bytes)
            .into_iter()
            .map(|(framing, reply)| {
                assert_eq!(reply["id"], Value::Null, "{reply}");
                let error = &reply["error"];
                let reason = error["data"].as_str().or(error["message"].as_str());
                (
                    framing,
                    error["code"].as_i64().unwrap(),
                    reason.unwrap().into(),
                )
            })
            .collect()
    }

    fn codes(errors: &[(Framing, i64, String)]) -> Vec<(Framing, i64)> {
        errors
            .iter()
            .map(|(framing, code, _)| (*framing, *code))
            .collect()
    }

    fn parse_error_for(reason: &str) -> (Framing, i64, String) {
        (Framing::ContentLength, PARSE_ERROR, reason.into())
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
            match decode(&buf, Cursor::START) {
                Decoded::Message { body, end, framing } => {
                    replies.push((framing, serde_json::from_slice(&buf[body]).unwrap()));
                    buf.drain(..end);
                }
                Decoded::Blank(len) => {
                    buf.drain(..len);
                }
                Decoded::Incomplete(_) => {
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
    async fn ndjson_drops_a_trailing_line_without_newline() {
        let input = [line(&ping(1)), ping(2).into_bytes()].concat();
        let (ids, errors) = ids_and_errors(input.as_slice()).await;
        assert_eq!(ids, [(Framing::Ndjson, 1)]);
        assert!(errors.is_empty(), "{errors:?}");
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

    #[test]
    fn content_length_values_are_digits_with_spaces_or_tabs_around_them_at_most() {
        let block = |value: &str| format!("Content-Length:{value}\r\n\r\n12345").into_bytes();
        for value in ["5", " 5", "5 ", "\t5\t", "  5  ", "05"] {
            let start = block(value).len() - 5;
            assert_eq!(
                decode(&block(value), Cursor::START),
                Decoded::Message {
                    body: start..start + 5,
                    end: start + 5,
                    framing: Framing::ContentLength,
                },
                "{value:?}"
            );
        }
        for value in [
            "+5", "-5", "5 5", "", " ", "0x5", "5_000", "5.0", "٥", "\u{a0}5",
        ] {
            assert_eq!(
                decode(&block(value), Cursor::START),
                Decoded::Malformed("Content-Length is not a byte count"),
                "{value:?}"
            );
        }
        let twice = b"Content-Length: 5\r\nContent-Length: 5\r\n\r\n12345";
        assert_eq!(
            decode(twice, Cursor::START),
            Decoded::Malformed("more than one Content-Length header")
        );
    }

    #[tokio::test]
    async fn content_length_counts_bytes_of_a_multibyte_body() {
        let json = r#"{"jsonrpc":"2.0","id":"héllo → 🙂","method":"ping"}"#;
        assert!(json.chars().count() < json.len());
        let input = [framed(json), framed(&ping(2))].concat();
        let (messages, _) = drive(input.as_slice()).await;
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
        let ids = ids_from(ByteAtATime(&input)).await;
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
    async fn a_header_block_that_cannot_be_used_is_answered_with_a_parse_error_and_closes_the_stream(
    ) {
        let blocks: [(&[u8], &str); 7] = [
            (
                b"Content-Length: abc\r\n\r\n",
                "Content-Length is not a byte count",
            ),
            (
                b"Content-Length: +5\r\n\r\n12345",
                "Content-Length is not a byte count",
            ),
            (
                b" Content-Length: 5\r\n\r\n12345",
                "a header line that starts with whitespace",
            ),
            (
                b"Content-Length: 5\r\nnot a header line\r\n\r\n12345",
                "a line that is not a header before the blank line",
            ),
            (
                b"Content-Length: 5\r\nContent-Length: 5\r\n\r\n12345",
                "more than one Content-Length header",
            ),
            (
                b"Content-Type: text/plain\r\n\r\n",
                "no Content-Length header",
            ),
            // The blank line is missing, so the next frame's header line
            // continues the block.
            (
                b"Content-Length: 10\r\n",
                "more than one Content-Length header",
            ),
        ];
        for (block, reason) in blocks {
            // The frames after the block are framed too: nothing but the
            // block's own end could separate them from its body.
            let input = [
                framed(&ping(1)),
                block.to_vec(),
                framed(&ping(2)),
                framed(&ping(3)),
            ]
            .concat();
            let block = String::from_utf8_lossy(block);
            let (ids, errors) = ids_and_errors(input.as_slice()).await;
            assert_eq!(ids, [(Framing::ContentLength, 1)], "{block}");
            assert_eq!(errors, [parse_error_for(reason)], "{block}");
            let (ids, errors) = ids_and_errors(ByteAtATime(&input)).await;
            assert_eq!(ids, [(Framing::ContentLength, 1)], "{block}");
            assert_eq!(errors, [parse_error_for(reason)], "{block}");
        }
    }

    #[tokio::test]
    async fn a_body_shorter_than_its_declared_length_loses_its_frame_but_not_the_frames_after_it() {
        // The declared length runs past the body, over the next frame and
        // into the one after it.
        let overrun = framed(&ping(3)).len() - 8;
        let input = [
            framed(&ping(1)),
            framed_as(&ping(2), ping(2).len() + overrun),
            framed(&ping(3)),
            framed(&ping(4)),
        ]
        .concat();
        let ids = [
            (Framing::ContentLength, 1),
            (Framing::ContentLength, 3),
            (Framing::ContentLength, 4),
        ];
        let (got, errors) = ids_and_errors(input.as_slice()).await;
        assert_eq!(got, ids);
        assert_eq!(codes(&errors), [(Framing::ContentLength, PARSE_ERROR)]);
        let (got, errors) = ids_and_errors(ByteAtATime(&input)).await;
        assert_eq!(got, ids);
        assert_eq!(codes(&errors), [(Framing::ContentLength, PARSE_ERROR)]);
    }

    #[tokio::test]
    async fn a_body_longer_than_its_declared_length_loses_its_frame_but_not_the_frames_after_it() {
        // The declared length stops short of the body, whose tail then sits
        // in front of the next frame's header line.
        let input = [
            framed(&ping(1)),
            framed_as(&ping(2), ping(2).len() - 1),
            framed(&ping(3)),
            framed(&ping(4)),
        ]
        .concat();
        let ids = [
            (Framing::ContentLength, 1),
            (Framing::ContentLength, 3),
            (Framing::ContentLength, 4),
        ];
        // One error for the body, one for its tail.
        let expected = [
            (Framing::ContentLength, PARSE_ERROR),
            (Framing::ContentLength, PARSE_ERROR),
        ];
        let (got, errors) = ids_and_errors(input.as_slice()).await;
        assert_eq!(got, ids);
        assert_eq!(codes(&errors), expected);
        let (got, errors) = ids_and_errors(ByteAtATime(&input)).await;
        assert_eq!(got, ids);
        assert_eq!(codes(&errors), expected);
    }

    #[tokio::test]
    async fn a_declared_length_that_cuts_into_the_next_header_loses_its_frame_but_not_the_frames_after_it(
    ) {
        // The declared length runs past the body into the next frame's
        // header line, by fewer bytes than the header's name has: whether
        // the rest of the name arrives with them or later, the frame is found.
        for overrun in 1..CONTENT_LENGTH.len() {
            let input = [
                framed(&ping(1)),
                framed_as(&ping(2), ping(2).len() + overrun),
                framed(&ping(3)),
                framed(&ping(4)),
            ]
            .concat();
            let ids = [
                (Framing::ContentLength, 1),
                (Framing::ContentLength, 3),
                (Framing::ContentLength, 4),
            ];
            // The whole stream in one read (it is shorter than the reader's
            // buffer), then one byte at a time.
            let (got, errors) = ids_and_errors(input.as_slice()).await;
            assert_eq!(got, ids, "overrun {overrun}");
            assert_eq!(
                codes(&errors),
                [(Framing::ContentLength, PARSE_ERROR)],
                "overrun {overrun}"
            );
            let (got, errors) = ids_and_errors(ByteAtATime(&input)).await;
            assert_eq!(got, ids, "overrun {overrun}");
            assert_eq!(
                codes(&errors),
                [(Framing::ContentLength, PARSE_ERROR)],
                "overrun {overrun}"
            );
        }
    }

    #[tokio::test]
    async fn bytes_kept_as_a_possible_header_are_dropped_unless_a_usable_header_block_follows() {
        // A body cut short right after `"c`: its last byte could start the
        // name of the next frame's header, but the rest of the body follows
        // it, not a header line. One error for the body, one for its tail.
        let json = r#"{"jsonrpc":"2.0","id":2,"method":"ping","params":{"c":1}}"#;
        let cut = json.find(r#""c""#).unwrap() + 2;
        let input = [framed(&ping(1)), framed_as(json, cut), framed(&ping(3))].concat();
        let ids = [(Framing::ContentLength, 1), (Framing::ContentLength, 3)];
        let expected = [
            (Framing::ContentLength, PARSE_ERROR),
            (Framing::ContentLength, PARSE_ERROR),
        ];
        let (got, errors) = ids_and_errors(input.as_slice()).await;
        assert_eq!(got, ids);
        assert_eq!(codes(&errors), expected);
        let (got, errors) = ids_and_errors(ByteAtATime(&input)).await;
        assert_eq!(got, ids);
        assert_eq!(codes(&errors), expected);

        // A declared length that overruns by one byte into a header block
        // without a `Content-Length`: the byte kept is dropped, and the block
        // closes the stream as it would have without the overrun.
        let input = [
            framed(&ping(1)),
            framed_as(&ping(2), ping(2).len() + 1),
            b"Cache: 1\r\n\r\n".to_vec(),
            framed(&ping(3)),
        ]
        .concat();
        let (got, errors) = ids_and_errors(input.as_slice()).await;
        assert_eq!(got, [(Framing::ContentLength, 1)]);
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert_eq!(errors[1], parse_error_for("no Content-Length header"));
    }

    #[tokio::test]
    async fn input_that_is_not_json_is_answered_with_a_parse_error_and_the_stream_goes_on() {
        // A line, then a framed body, neither JSON; the body is last, so its
        // error is still pending when the stream ends.
        let input = [
            line("not json"),
            line(&ping(1)),
            framed(&ping(2)),
            framed("not json either"),
        ]
        .concat();
        let (ids, errors) = ids_and_errors(input.as_slice()).await;
        assert_eq!(ids, [(Framing::Ndjson, 1), (Framing::ContentLength, 2)]);
        assert_eq!(
            codes(&errors),
            [
                (Framing::Ndjson, PARSE_ERROR),
                (Framing::ContentLength, PARSE_ERROR),
            ]
        );
    }

    #[tokio::test]
    async fn a_header_flood_is_rejected_at_the_header_cap_without_reading_on() {
        let padded = |head: &str, size: usize| [head.as_bytes(), &b"a".repeat(size)].concat();
        let lines = |size: usize| b"X: y\r\n".repeat(size / 6);
        let floods = [
            padded("X-Pad: ", 2 << 20),
            padded("Content-Length: 5\r\nX-Pad: ", 2 << 20),
            lines(2 << 20),
            lines(20 << 20),
            padded("X-Pad: ", 20 << 20),
        ];
        for flood in floods {
            let input = [framed(&ping(1)), flood, framed(&ping(2))].concat();
            let started = Instant::now();
            let mut transport = StdioTransport::new(input.as_slice(), Vec::new());
            assert_eq!(request_id(&transport.receive().await.unwrap()), 1);
            assert!(transport.receive().await.is_none());
            let elapsed = started.elapsed();
            // Rejected on the read that takes the block past the cap: the
            // rest of the flood is neither read nor scanned.
            let read = input.len() - transport.read.get_ref().len();
            assert!(read < 4 * MAX_HEADER_BYTES, "{read} bytes read");
            assert!(transport.buf.len() <= 2 * MAX_HEADER_BYTES);
            assert!(elapsed < Duration::from_millis(500), "{elapsed:?}");
            let written = transport.write.lock().await.stream.take().unwrap();
            assert_eq!(errors_in(&written), [parse_error_for(HEADERS_TOO_LONG)]);
        }
    }

    #[tokio::test]
    async fn a_header_block_of_many_short_lines_above_the_cap_closes_the_stream() {
        let lines = b"X: y\r\n".repeat(MAX_HEADER_BYTES / 6 + 1);
        let input = [framed(&ping(1)), lines, framed(&ping(2))].concat();
        let (ids, errors) = ids_and_errors(input.as_slice()).await;
        assert_eq!(ids, [(Framing::ContentLength, 1)]);
        assert_eq!(errors, [parse_error_for(HEADERS_TOO_LONG)]);
        let (ids, errors) = ids_and_errors(ByteAtATime(&input)).await;
        assert_eq!(ids, [(Framing::ContentLength, 1)]);
        assert_eq!(errors, [parse_error_for(HEADERS_TOO_LONG)]);
    }

    #[tokio::test]
    async fn a_header_block_at_the_cap_is_accepted_and_one_byte_over_is_not() {
        // A header block of exactly `size` bytes, blank line included.
        let block = |json: &str, size: usize| {
            let head = format!("Content-Length: {}\r\nX-Pad: ", json.len());
            let pad = "x".repeat(size - head.len() - b"\r\n\r\n".len());
            format!("{head}{pad}\r\n\r\n{json}").into_bytes()
        };
        let at_cap = [block(&ping(1), MAX_HEADER_BYTES), line(&ping(2))].concat();
        let ids = [(Framing::ContentLength, 1), (Framing::Ndjson, 2)];
        assert_eq!(received_ids(&at_cap).await, ids);
        assert_eq!(ids_from(ByteAtATime(&at_cap)).await, ids);

        let over = [framed(&ping(1)), block(&ping(2), MAX_HEADER_BYTES + 1)].concat();
        let (ids, errors) = ids_and_errors(over.as_slice()).await;
        assert_eq!(ids, [(Framing::ContentLength, 1)]);
        assert_eq!(errors, [parse_error_for(HEADERS_TOO_LONG)]);
    }

    #[tokio::test]
    async fn a_frame_cut_off_by_the_end_of_the_stream_is_dropped() {
        let tails: [&[u8]; 2] = [
            b"Content-Length: 10\r\n",
            b"Content-Length: 100\r\n\r\n{\"jsonrpc\"",
        ];
        for tail in tails {
            let input = [framed(&ping(1)), tail.to_vec()].concat();
            let (ids, errors) = ids_and_errors(input.as_slice()).await;
            assert_eq!(ids, [(Framing::ContentLength, 1)]);
            assert!(errors.is_empty(), "{errors:?}");
        }
    }

    #[tokio::test]
    async fn a_line_at_the_limit_is_delivered_and_one_byte_over_is_dropped() {
        let at_limit = sized_line(1, MAX_MESSAGE_BYTES);
        let over = sized_line(2, MAX_MESSAGE_BYTES + 1);
        let input = [at_limit.clone(), over, line(&ping(3))].concat();
        // A read never spans a `chain` boundary, and a part shorter than the
        // reader's buffer arrives in one read: the read that takes the second
        // line past the limit holds its newline too.
        let split = at_limit.len() + MAX_MESSAGE_BYTES - 100;
        let read = (&input[..split]).chain(&input[split..]);
        assert_eq!(
            ids_from(read).await,
            [(Framing::Ndjson, 1), (Framing::Ndjson, 3)]
        );
    }

    #[tokio::test]
    async fn an_oversized_line_whose_newline_arrives_later_is_skipped_through_its_end() {
        let over = sized_line(1, MAX_MESSAGE_BYTES + 2);
        let input = [over, line(&ping(2))].concat();
        // The read that takes the line past the limit stops short of its
        // newline, which the read after it brings.
        let crossing = MAX_MESSAGE_BYTES + 1;
        let read = (&input[..crossing - 100])
            .chain(&input[crossing - 100..crossing])
            .chain(&input[crossing..]);
        assert_eq!(ids_from(read).await, [(Framing::Ndjson, 2)]);
    }

    #[tokio::test]
    async fn leading_carriage_returns_count_toward_the_line_limit() {
        // A run of `\r` is not blank lines but part of the line after it,
        // which is dropped past the limit and delivered under it. Blank
        // lines, `\n` or `\r\n`, are skipped as ever.
        let over = [b"\r".repeat(MAX_MESSAGE_BYTES), line(&ping(1))].concat();
        let input = [
            over,
            b"\r\r".to_vec(),
            line(&ping(2)),
            b"\r\n\n".to_vec(),
            line(&ping(3)),
        ]
        .concat();
        assert_eq!(
            received_ids(&input).await,
            [(Framing::Ndjson, 2), (Framing::Ndjson, 3)]
        );
    }

    #[tokio::test]
    async fn a_framed_body_above_the_limit_is_skipped_and_the_next_frames_delivered() {
        let input = [
            framed(&"x".repeat(MAX_MESSAGE_BYTES + 1)),
            framed(&ping(2)),
            framed(&ping(3)),
        ]
        .concat();
        let ids = [(Framing::ContentLength, 2), (Framing::ContentLength, 3)];
        // The whole stream in one read.
        let mut transport = StdioTransport::new(&[][..], Vec::new());
        transport.buf = input.clone();
        assert_eq!(request_id(&transport.receive().await.unwrap()), 2);
        assert_eq!(request_id(&transport.receive().await.unwrap()), 3);
        assert!(transport.receive().await.is_none());
        // In chunks.
        assert_eq!(received_ids(&input).await, ids);
        // The end of the body and the frames after it one byte at a time.
        let tail = input.len() - 2 * framed(&ping(2)).len() - 16;
        let read = (&input[..tail]).chain(ByteAtATime(&input[tail..]));
        assert_eq!(ids_from(read).await, ids);
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
    async fn json_that_is_not_a_message_is_answered_before_the_end_of_the_stream_is_reported() {
        let input = [line(&ping(1)), framed(r#"{"hello":"world"}"#)].concat();
        let (mut client, server) = tokio::io::duplex(64 * 1024);
        let (_, write) = tokio::io::split(server);
        let mut transport = StdioTransport::new(input.as_slice(), write);
        assert_eq!(request_id(&transport.receive().await.unwrap()), 1);
        // The reply is still pending when the stream ends: it is written
        // before `receive` reports the end, so closing cannot lose it.
        assert!(transport.receive().await.is_none());
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

    #[tokio::test]
    async fn stalled_replies_hold_back_reading_until_each_is_written() {
        let input = [
            line("not json"),
            line(r#"{"hello":"world"}"#),
            line(&ping(1)),
            line(&ping(2)),
        ]
        .concat();
        // Room for part of a reply only, and nobody reading it yet.
        let (mut client, server) = tokio::io::duplex(8);
        let (_, write) = tokio::io::split(server);
        let mut transport = StdioTransport::new(ByteAtATime(&input), write);
        let stalled = tokio::time::timeout(Duration::from_millis(50), transport.receive()).await;
        assert!(stalled.is_err(), "{stalled:?}");
        // Nothing past the message being answered has been read.
        let unread = input.len() - line("not json").len();
        assert_eq!(transport.read.get_ref().0.len(), unread);
        assert!(transport.pending.is_some());

        // Reading the replies lets the transport go on, one reply at a time;
        // the first arrives whole although its write was cancelled part way.
        let (replies, message) = tokio::join!(read_replies(&mut client, 2), transport.receive());
        assert_eq!(request_id(&message.unwrap()), 1);
        assert_eq!(replies[0].0, Framing::Ndjson);
        assert_eq!(replies[0].1["error"]["code"], json!(PARSE_ERROR));
        assert_eq!(replies[1].0, Framing::Ndjson);
        assert_eq!(replies[1].1["error"]["code"], json!(INVALID_REQUEST));
        assert!(transport.pending.is_none());
        assert_eq!(request_id(&transport.receive().await.unwrap()), 2);
    }

    #[tokio::test]
    async fn a_response_sent_while_an_error_reply_is_part_written_follows_it_whole() {
        for framing in [Framing::Ndjson, Framing::ContentLength] {
            let encode = |json: &str| match framing {
                Framing::Ndjson => line(json),
                Framing::ContentLength => framed(json),
            };
            let input = [encode(&ping(1)), encode("not json")].concat();
            // Room for part of the error reply only, and nobody reading it yet.
            let (mut client, server) = tokio::io::duplex(8);
            let (_, write) = tokio::io::split(server);
            let mut transport = StdioTransport::new(ByteAtATime(&input), write);
            assert_eq!(request_id(&transport.receive().await.unwrap()), 1);
            // The error reply's write stalls part way, and `receive` is
            // dropped there: the rest of the frame is left with the writer.
            let stalled =
                tokio::time::timeout(Duration::from_millis(50), transport.receive()).await;
            assert!(stalled.is_err(), "{stalled:?}");
            assert!(matches!(transport.pending, Some(Pending::Writing)));
            assert!(transport.write.lock().await.in_flight.is_some());

            // The response to request 1 finishes the error frame before
            // writing its own, once the client reads.
            let sent = transport.send(reply(1));
            let server = async {
                sent.await.unwrap();
                assert!(transport.receive().await.is_none());
                assert!(transport.pending.is_none());
                transport.close().await.unwrap();
            };
            let drain = async {
                let mut out = Vec::new();
                client.read_to_end(&mut out).await.unwrap();
                out
            };
            let (out, ()) = tokio::join!(drain, server);
            let frames = messages_in(&out);
            assert_eq!(frames.len(), 2, "{frames:?}");
            assert_eq!(frames[0].0, framing, "{frames:?}");
            assert_eq!(frames[0].1["id"], Value::Null, "{frames:?}");
            assert_eq!(frames[0].1["error"]["code"], json!(PARSE_ERROR));
            assert_eq!(frames[1].0, framing, "{frames:?}");
            assert_eq!(frames[1].1["id"], json!(1), "{frames:?}");
            assert_eq!(frames[1].1["result"], json!({}), "{frames:?}");
        }
    }

    #[tokio::test]
    async fn a_cancelled_request_is_forgotten_and_later_replies_keep_their_framing() {
        let input = [
            framed(&ping(1)),
            line(&ping(2)),
            framed(&ping(3)),
            line(&ping(4)),
            line(&cancel(1)),
            framed(&cancel(2)),
        ]
        .concat();
        let (mut client, server) = tokio::io::duplex(64 * 1024);
        let (_, write) = tokio::io::split(server);
        let mut transport = StdioTransport::new(input.as_slice(), write);
        for _ in 0..4 {
            transport.receive().await.unwrap();
        }
        assert_eq!(transport.requests.len(), 4);
        for _ in 0..2 {
            transport.receive().await.unwrap();
        }
        assert_eq!(
            transport.requests,
            [
                (RequestId::Number(3), Framing::ContentLength),
                (RequestId::Number(4), Framing::Ndjson),
            ]
        );
        assert!(transport.receive().await.is_none());

        // Each in its own framing, not the framing last seen.
        transport.send(reply(4)).await.unwrap();
        transport.send(reply(3)).await.unwrap();
        transport.close().await.unwrap();
        let replies = read_replies(&mut client, 2).await;
        assert_eq!(replies[0].0, Framing::Ndjson);
        assert_eq!(replies[0].1["id"], json!(4));
        assert_eq!(replies[1].0, Framing::ContentLength);
        assert_eq!(replies[1].1["id"], json!(3));
        assert!(transport.requests.is_empty());
    }

    #[tokio::test]
    async fn a_request_past_the_count_budget_is_answered_busy_until_a_reply_makes_room() {
        // A framed request and newline-delimited ones fill the budget; one
        // more, newline-delimited, is over it.
        let last = MAX_OUTSTANDING_REQUESTS as i64;
        let mut input = framed(&ping(1));
        for id in 2..=last {
            input.extend(line(&ping(id)));
        }
        input.extend(line(&ping(last + 1)));
        let (mut stdin, read) = tokio::io::duplex(input.len());
        let (mut client, server) = tokio::io::duplex(64 * 1024);
        let (_, write) = tokio::io::split(server);
        let mut transport = StdioTransport::new(read, write);
        stdin.write_all(&input).await.unwrap();
        for id in 1..=last {
            assert_eq!(request_id(&transport.receive().await.unwrap()), id);
        }
        // The request over the budget is answered in its framing, not
        // delivered: `receive` waits for more input instead.
        let waiting = tokio::time::timeout(Duration::from_millis(50), transport.receive()).await;
        assert!(waiting.is_err(), "{waiting:?}");
        assert_eq!(transport.requests.len(), MAX_OUTSTANDING_REQUESTS);
        let replies = read_replies(&mut client, 1).await;
        assert_eq!(replies[0].0, Framing::Ndjson);
        assert_eq!(replies[0].1["id"], json!(last + 1));
        assert_eq!(replies[0].1["error"]["code"], json!(SERVER_BUSY.0));

        // Request 1 is still tracked: its reply is in its own framing, not
        // the one last seen, and makes room for a request.
        transport.send(reply(1)).await.unwrap();
        let replies = read_replies(&mut client, 1).await;
        assert_eq!(replies[0].0, Framing::ContentLength);
        assert_eq!(replies[0].1["id"], json!(1));
        assert_eq!(transport.requests.len(), MAX_OUTSTANDING_REQUESTS - 1);
        stdin.write_all(&framed(&ping(last + 2))).await.unwrap();
        assert_eq!(request_id(&transport.receive().await.unwrap()), last + 2);
        assert_eq!(transport.requests.len(), MAX_OUTSTANDING_REQUESTS);
        assert_eq!(
            transport.requests.back(),
            Some(&(RequestId::Number(last + 2), Framing::ContentLength))
        );
    }

    #[tokio::test]
    async fn a_request_past_the_id_byte_budget_is_answered_busy_until_a_reply_makes_room() {
        // Four framed requests whose string ids fill the budget between
        // them, then a newline-delimited one whose one-byte id is over it.
        let quarter = MAX_OUTSTANDING_ID_BYTES / 4;
        let big = |n: usize| format!("{n}{}", "x".repeat(quarter - 1));
        let request = |id: &str| format!(r#"{{"jsonrpc":"2.0","id":"{id}","method":"ping"}}"#);
        let mut input = Vec::new();
        for n in 1..=4 {
            input.extend(framed(&request(&big(n))));
        }
        input.extend(line(&request("5")));
        let (mut stdin, read) = tokio::io::duplex(input.len());
        let (mut client, server) = tokio::io::duplex(64 * 1024);
        let (_, write) = tokio::io::split(server);
        let mut transport = StdioTransport::new(read, write);
        stdin.write_all(&input).await.unwrap();
        for _ in 1..=4 {
            transport.receive().await.unwrap();
        }
        assert_eq!(transport.id_bytes, MAX_OUTSTANDING_ID_BYTES);
        let waiting = tokio::time::timeout(Duration::from_millis(50), transport.receive()).await;
        assert!(waiting.is_err(), "{waiting:?}");
        assert_eq!(transport.requests.len(), 4);
        let replies = read_replies(&mut client, 1).await;
        assert_eq!(replies[0].0, Framing::Ndjson);
        assert_eq!(replies[0].1["id"], json!("5"));
        assert_eq!(replies[0].1["error"]["code"], json!(SERVER_BUSY.0));

        // The reply to the first request frees its id's bytes.
        let answered = ServerJsonRpcMessage::response(
            ServerResult::empty(()),
            RequestId::String(big(1).into()),
        );
        transport.send(answered).await.unwrap();
        let replies = read_replies(&mut client, 1).await;
        assert_eq!(replies[0].0, Framing::ContentLength);
        assert_eq!(replies[0].1["id"], json!(big(1)));
        assert_eq!(transport.id_bytes, MAX_OUTSTANDING_ID_BYTES - quarter);
        stdin.write_all(&line(&request("6"))).await.unwrap();
        let JsonRpcMessage::Request(admitted) = transport.receive().await.unwrap() else {
            panic!("not a request");
        };
        assert_eq!(admitted.id, RequestId::String("6".into()));
        assert_eq!(transport.requests.len(), 4);
        assert_eq!(transport.id_bytes, MAX_OUTSTANDING_ID_BYTES - quarter + 1);
    }

    #[tokio::test]
    async fn a_request_repeating_an_outstanding_id_is_answered_invalid_and_takes_no_room() {
        // A request in one framing, then more repeats of its id than the
        // budget holds in the other, with a fresh request among them and
        // one after them. The string id makes the budget's bytes visible.
        let request = |id: &str| format!(r#"{{"jsonrpc":"2.0","id":"{id}","method":"ping"}}"#);
        let answer = |id: &str| {
            ServerJsonRpcMessage::response(ServerResult::empty(()), RequestId::String(id.into()))
        };
        for (original, other) in [
            (Framing::ContentLength, Framing::Ndjson),
            (Framing::Ndjson, Framing::ContentLength),
        ] {
            let encode = |json: &str, framing| match framing {
                Framing::Ndjson => line(json),
                Framing::ContentLength => framed(json),
            };
            let mut input = encode(&request("a"), original);
            for n in 0..=MAX_OUTSTANDING_REQUESTS {
                if n == MAX_OUTSTANDING_REQUESTS / 2 {
                    input.extend(encode(&ping(2), other));
                }
                input.extend(encode(&request("a"), other));
            }
            input.extend(encode(&ping(3), other));
            let (mut stdin, read) = tokio::io::duplex(input.len());
            let (mut client, server) = tokio::io::duplex(1 << 20);
            let (_, write) = tokio::io::split(server);
            let mut transport = StdioTransport::new(read, write);
            stdin.write_all(&input).await.unwrap();
            transport.receive().await.unwrap();
            assert_eq!(request_id(&transport.receive().await.unwrap()), 2);
            assert_eq!(request_id(&transport.receive().await.unwrap()), 3);
            let waiting =
                tokio::time::timeout(Duration::from_millis(50), transport.receive()).await;
            assert!(waiting.is_err(), "{waiting:?}");
            assert_eq!(
                transport.requests,
                [
                    (RequestId::String("a".into()), original),
                    (RequestId::Number(2), other),
                    (RequestId::Number(3), other),
                ]
            );
            assert_eq!(transport.id_bytes, 1);
            let replies = read_replies(&mut client, MAX_OUTSTANDING_REQUESTS + 1).await;
            for (framing, reply) in &replies {
                assert_eq!(*framing, other, "{reply}");
                assert_eq!(reply["id"], json!("a"), "{reply}");
                assert_eq!(reply["error"]["code"], json!(INVALID_REQUEST), "{reply}");
                assert_eq!(
                    reply["error"]["message"],
                    json!("Duplicate request id"),
                    "{reply}"
                );
            }

            // The repeats changed nothing: the original is answered in its
            // own framing, not the one last seen, and so is the fresh one.
            transport.send(reply(3)).await.unwrap();
            transport.send(answer("a")).await.unwrap();
            let replies = read_replies(&mut client, 2).await;
            assert_eq!(replies[0].0, other);
            assert_eq!(replies[0].1["id"], json!(3));
            assert_eq!(replies[1].0, original);
            assert_eq!(replies[1].1["id"], json!("a"));

            // An id is free again once its request is answered or cancelled.
            let reuse = [
                encode(&request("a"), other),
                encode(&cancel(2), other),
                encode(&ping(2), original),
            ]
            .concat();
            stdin.write_all(&reuse).await.unwrap();
            transport.receive().await.unwrap();
            transport.receive().await.unwrap();
            assert_eq!(request_id(&transport.receive().await.unwrap()), 2);
            assert_eq!(
                transport.requests,
                [
                    (RequestId::String("a".into()), other),
                    (RequestId::Number(2), original),
                ]
            );
            assert_eq!(transport.id_bytes, 1);
        }
    }

    const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"framing-test","version":"0"}}}"#;
    const INITIALIZED: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    const TOOLS_LIST: &str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;

    fn server() -> crate::AreteMcp {
        crate::AreteMcp::with_agents(
            crate::protocol_agent::ProtocolAgent::new(None),
            crate::sandbox::SandboxClient::from_values(None, None, None),
        )
    }

    /// Serve a session over in-memory streams that `input` opens: the first
    /// `count` replies, each with its framing.
    async fn serve(input: &[u8], count: usize) -> Vec<(Framing, Value)> {
        let (mut client, server_end) = tokio::io::duplex(1 << 20);
        let (read, write) = tokio::io::split(server_end);
        client.write_all(input).await.unwrap();
        let running = server()
            .serve(StdioTransport::new(read, write))
            .await
            .unwrap();
        let replies = read_replies(&mut client, count).await;
        drop(client);
        running.waiting().await.unwrap();
        replies
    }

    fn session(framing: Framing) -> Vec<u8> {
        [INITIALIZE, INITIALIZED, TOOLS_LIST]
            .into_iter()
            .flat_map(|json| match framing {
                Framing::Ndjson => line(json),
                Framing::ContentLength => framed(json),
            })
            .collect()
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
        let replies = serve(&session(Framing::Ndjson), 2).await;
        assert_handshake_and_tools(&replies, Framing::Ndjson);
    }

    #[tokio::test]
    async fn serves_a_content_length_framed_session() {
        let replies = serve(&session(Framing::ContentLength), 2).await;
        assert_handshake_and_tools(&replies, Framing::ContentLength);
    }

    #[tokio::test]
    async fn a_session_answers_the_requests_after_a_frame_whose_length_overran_them() {
        // Into the next frame's header line by fewer bytes than its name
        // has, and well past it.
        for overrun in (1..CONTENT_LENGTH.len()).chain([30]) {
            let input = [
                framed(INITIALIZE),
                framed(INITIALIZED),
                framed_as(&ping(3), ping(3).len() + overrun),
                framed(TOOLS_LIST),
            ]
            .concat();
            let replies = serve(&input, 3).await;
            assert!(
                replies
                    .iter()
                    .all(|(seen, _)| *seen == Framing::ContentLength),
                "overrun {overrun}: {replies:?}"
            );
            assert_eq!(replies[0].1["id"], json!(1), "overrun {overrun}");
            assert_eq!(replies[1].1["id"], Value::Null, "overrun {overrun}");
            assert_eq!(replies[1].1["error"]["code"], json!(PARSE_ERROR));
            assert_eq!(replies[2].1["id"], json!(2), "overrun {overrun}");
            assert!(
                replies[2].1["result"]["tools"].is_array(),
                "overrun {overrun}: {replies:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_session_ends_after_the_parse_error_for_a_header_block_that_cannot_be_used() {
        let input = [
            framed(INITIALIZE),
            framed(INITIALIZED),
            b"Content-Length: abc\r\n\r\n".to_vec(),
            framed(TOOLS_LIST),
        ]
        .concat();
        let (mut client, server_end) = tokio::io::duplex(1 << 20);
        let (read, write) = tokio::io::split(server_end);
        client.write_all(&input).await.unwrap();
        let running = server()
            .serve(StdioTransport::new(read, write))
            .await
            .unwrap();
        // The server ends the session on its own, with the error written
        // before it does.
        running.waiting().await.unwrap();
        let mut out = Vec::new();
        client.read_to_end(&mut out).await.unwrap();
        let replies = messages_in(&out);
        assert_eq!(replies.len(), 2, "{replies:?}");
        assert_eq!(replies[0].1["id"], json!(1));
        assert_eq!(replies[1].0, Framing::ContentLength);
        assert_eq!(replies[1].1["error"]["code"], json!(PARSE_ERROR));
        assert_eq!(
            replies[1].1["error"]["data"],
            json!("Content-Length is not a byte count")
        );
    }
}
