//! `arete.raw-capture/v1`: a recorded window of raw Solana updates for a set
//! of programs, for replaying a stack offline.
//!
//! A capture file (`.arcap`) is zstd-compressed JSON lines:
//!
//! 1. One header line ([`CaptureHeader`], `"type": "header"`): the programs,
//!    cluster and genesis hash, the slot range, the producer, how faithful
//!    the starting state is, the record counts and the content hash.
//! 2. The starting account state: `"type": "state"` lines, each an account
//!    update ([`CaptureUpdate`]) delivered before the window's updates.
//! 3. The window's updates in delivery order (slot, then position in the
//!    slot): `"type": "update"` lines.
//!
//! Every update has the shape a Yellowstone (geyser) subscription for the
//! programs feeds a stack's runtime: an account write with its full data, or
//! a transaction that includes one of the programs, with its signer and
//! writable flags, top-level instructions, the instructions of the captured
//! programs (top-level and inner, with their instruction-tree paths) and its
//! log messages. Byte fields are lowercase hex.
//!
//! The content hash is `sha256:<hex>` over the bytes of every line after the
//! header, each with its trailing `\n`, exactly as stored. [`read_capture`]
//! checks it and the counts.
//!
//! Files written before the header existed (plain JSON lines of updates, no
//! header) are read too: [`read_capture`] reports them with
//! [`CaptureHeader::legacy`] and no hash check.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const RAW_CAPTURE_SCHEMA: &str = "arete.raw-capture/v1";
/// The file extension captures use.
pub const RAW_CAPTURE_EXTENSION: &str = "arcap";
/// zstd's frame magic, little-endian on disk.
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];
const ZSTD_LEVEL: i32 = 3;

/// How the capture was produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureProducer {
    /// The program that wrote the file, e.g. `a4`.
    pub tool: String,
    pub version: String,
    /// `rpc` (signatures and transactions read from a JSON-RPC node),
    /// `fork` (a local fork's geyser stream, recorded as it happened),
    /// `arete` (a hosted per-program snapshot) or `import` (converted from
    /// another file).
    pub method: String,
    /// Where the data came from, without credentials: an RPC host, `fork`, a
    /// file name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// RFC 3339.
    pub created_at: String,
}

/// The slots the capture's updates span (inclusive).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotRange {
    pub first: u64,
    pub last: u64,
}

/// The window the producer was asked for, which can be wider than the
/// updates it found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureWindow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_slot: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_slot: Option<u64>,
    /// Unix seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_time: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_time: Option<i64>,
}

/// How faithful the capture's starting account state is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StateFidelity {
    /// No starting state: replay starts from empty state. Right for a
    /// recording that began before the activity it holds (a fork recording
    /// from the fork's start).
    None,
    /// Each account as it was when the window began.
    Exact,
    /// Each account as a node returned it when the capture was made, at
    /// [`StartingState::slot`], which is after the window: values can be
    /// newer than the window's transactions. JSON-RPC serves no historical
    /// account data, so this is the best an RPC capture can do.
    Current,
}

/// The starting account state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartingState {
    pub fidelity: StateFidelity,
    /// The slot the accounts were read at, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<u64>,
    /// How the accounts were chosen: `touched` (accounts the window's
    /// transactions reference that the captured programs own), `explicit`
    /// (`--accounts`), `touched+explicit`, or `none`.
    pub selection: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl StartingState {
    pub fn none() -> Self {
        Self {
            fidelity: StateFidelity::None,
            slot: None,
            selection: "none".into(),
            note: None,
        }
    }
}

/// How updates within one slot are ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureOrdering {
    /// As a geyser stream delivered them.
    Stream,
    /// Transactions by their index in the block.
    Block,
    /// Transactions in the order the node listed their signatures, which
    /// is usually, not provably, block order; `txn_index` is that position.
    /// With several programs, a slot holds each program's transactions in
    /// turn (the header's `limits` say how many slots), not interleaved.
    Approximate,
}

/// Record counts, filled in when the capture is finished.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureCounts {
    /// Starting-state account records.
    pub state: u64,
    /// Window records (accounts and transactions).
    pub updates: u64,
    pub transactions: u64,
    pub accounts: u64,
}

/// The first line of a capture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureHeader {
    pub schema: String,
    /// The captured programs (base58).
    pub programs: Vec<String>,
    /// `mainnet-beta`, `devnet`, `testnet`, `localnet`, `fork` or `unknown`.
    pub cluster: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genesis_hash: Option<String>,
    /// The slots the records span; zero when there are none.
    pub slot_range: SlotRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<CaptureWindow>,
    pub producer: CaptureProducer,
    pub starting_state: StartingState,
    pub ordering: CaptureOrdering,
    pub counts: CaptureCounts,
    /// `sha256:<hex>` over every line after the header. Empty only for a
    /// legacy file.
    pub content_hash: String,
    /// What the producer could not capture exactly, for a reader to weigh.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limits: Vec<String>,
}

impl CaptureHeader {
    pub fn new(
        programs: Vec<String>,
        cluster: impl Into<String>,
        producer: CaptureProducer,
    ) -> Self {
        Self {
            schema: RAW_CAPTURE_SCHEMA.into(),
            programs,
            cluster: cluster.into(),
            genesis_hash: None,
            slot_range: SlotRange::default(),
            window: None,
            producer,
            starting_state: StartingState::none(),
            ordering: CaptureOrdering::Stream,
            counts: CaptureCounts::default(),
            content_hash: String::new(),
            limits: Vec::new(),
        }
    }

    /// The header a headerless (pre-v1) JSON-lines file is read with.
    pub fn legacy(programs: Vec<String>) -> Self {
        let mut header = Self::new(
            programs,
            "unknown",
            CaptureProducer {
                tool: "unknown".into(),
                version: String::new(),
                method: "import".into(),
                source: None,
                created_at: String::new(),
            },
        );
        header
            .limits
            .push("legacy raw capture without a header: no content hash".into());
        header
    }

    pub fn is_legacy(&self) -> bool {
        self.content_hash.is_empty()
    }
}

/// One raw update, in the shape a stack's runtime consumes from geyser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureUpdate {
    /// Position in the stream; the reader renumbers nothing.
    pub seq: u64,
    pub slot: u64,
    /// When the update was observed, Unix milliseconds: the stream's
    /// receive time for a fork recording, the block time for an RPC
    /// capture (0 when the node returned none; the header's `limits` count
    /// those). Replays stamp entity timestamps with it; real-time playback
    /// paces by slot.
    pub received_at: i64,
    pub kind: CaptureUpdateKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CaptureUpdateKind {
    Account(CaptureAccount),
    Transaction(CaptureTransaction),
}

impl CaptureUpdate {
    pub fn signature(&self) -> Option<&str> {
        match &self.kind {
            CaptureUpdateKind::Account(account) => account.txn_signature.as_deref(),
            CaptureUpdateKind::Transaction(transaction) => Some(&transaction.signature),
        }
    }

    pub fn kind_name(&self) -> &'static str {
        match &self.kind {
            CaptureUpdateKind::Account(_) => "account",
            CaptureUpdateKind::Transaction(_) => "transaction",
        }
    }
}

/// An account write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureAccount {
    pub pubkey: String,
    pub owner: String,
    #[serde(with = "hex_bytes")]
    pub data: Vec<u8>,
    pub lamports: u64,
    pub write_version: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub txn_signature: Option<String>,
}

/// Which message keys sign and which are writable.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureAccountFlags {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writable: Vec<String>,
}

impl CaptureAccountFlags {
    pub fn is_empty(&self) -> bool {
        self.signers.is_empty() && self.writable.is_empty()
    }

    /// From a message header's three counts over its static keys and the
    /// keys its lookup tables loaded as writable (which never sign).
    pub fn from_message(
        num_required_signatures: usize,
        num_readonly_signed: usize,
        num_readonly_unsigned: usize,
        static_keys: &[String],
        loaded_writable: &[String],
    ) -> Self {
        let signer_count = num_required_signatures.min(static_keys.len());
        let writable_signers = signer_count.saturating_sub(num_readonly_signed);
        let writable_unsigned_end = static_keys
            .len()
            .saturating_sub(num_readonly_unsigned)
            .max(signer_count);
        Self {
            signers: static_keys[..signer_count].to_vec(),
            writable: static_keys[..writable_signers]
                .iter()
                .chain(&static_keys[signer_count..writable_unsigned_end])
                .chain(loaded_writable)
                .cloned()
                .collect(),
        }
    }
}

/// A transaction that includes one of the captured programs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CaptureTransaction {
    #[serde(default, skip_serializing_if = "CaptureAccountFlags::is_empty")]
    pub account_flags: CaptureAccountFlags,
    /// `{"version": "legacy" | 0 | 1, "config"?: {...}}`, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_metadata: Option<Value>,
    pub signature: String,
    /// Present only for a failed transaction; producers skip those, as a
    /// stack's subscription does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Position in the block (see [`CaptureOrdering`]).
    #[serde(default)]
    pub txn_index: u64,
    /// Every top-level instruction, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub top_level_instructions: Vec<CaptureTopLevelInstruction>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log_messages: Vec<String>,
    /// The captured programs' instructions, top-level then inner.
    pub instructions: Vec<CaptureInstruction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureTopLevelInstruction {
    pub program_id: String,
    #[serde(with = "hex_bytes")]
    pub data: Vec<u8>,
    #[serde(default)]
    pub accounts: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaptureInstruction {
    pub program_id: String,
    #[serde(with = "hex_bytes")]
    pub data: Vec<u8>,
    pub accounts: Vec<String>,
    #[serde(default)]
    pub is_inner: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_level_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_index: Option<usize>,
    /// Zero-based instruction-tree path, e.g. `2` or `2.0.1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ix_path: Option<String>,
}

/// One line after the header.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum CaptureRecord {
    /// Starting account state, delivered before the window.
    State { update: CaptureUpdate },
    /// An update in the window.
    Update { update: CaptureUpdate },
}

impl CaptureRecord {
    pub fn update(&self) -> &CaptureUpdate {
        match self {
            Self::State { update } | Self::Update { update } => update,
        }
    }

    pub fn into_update(self) -> CaptureUpdate {
        match self {
            Self::State { update } | Self::Update { update } => update,
        }
    }

    pub fn is_state(&self) -> bool {
        matches!(self, Self::State { .. })
    }
}

#[derive(Serialize)]
struct HeaderLine<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(flatten)]
    header: &'a CaptureHeader,
}

/// The header line read back; its `type` is checked before parsing.
#[derive(Deserialize)]
struct HeaderLineOwned {
    #[serde(flatten)]
    header: CaptureHeader,
}

/// Errors reading or writing a capture.
#[derive(Debug)]
pub enum CaptureError {
    Io(std::io::Error),
    Invalid(String),
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "{error}"),
            Self::Invalid(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for CaptureError {}

impl From<std::io::Error> for CaptureError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

fn invalid(message: impl Into<String>) -> CaptureError {
    CaptureError::Invalid(message.into())
}

/// Writes a capture: records go to a temporary body file as they arrive
/// (hashed and counted), and [`CaptureWriter::finish`] writes the header
/// with the totals followed by the body, compressed, then renames it into
/// place. Records must be added state first, then updates, in delivery
/// order.
///
/// The temporary files never outlive the writer: dropping it without
/// finishing (an interrupted capture), or a failed finish, removes them, and
/// no capture file is written.
pub struct CaptureWriter {
    path: PathBuf,
    body_path: PathBuf,
    staging_path: PathBuf,
    body: Option<BufWriter<File>>,
    hasher: Sha256,
    header: CaptureHeader,
    seen_update: bool,
}

impl CaptureWriter {
    /// `header` carries everything but the counts, slot range and hash.
    pub fn create(path: impl Into<PathBuf>, header: CaptureHeader) -> Result<Self, CaptureError> {
        let path = path.into();
        let body_path = sibling(&path, ".body.tmp");
        let staging_path = sibling(&path, ".tmp");
        let body = BufWriter::new(File::create(&body_path)?);
        Ok(Self {
            path,
            body_path,
            staging_path,
            body: Some(body),
            hasher: Sha256::new(),
            header,
            seen_update: false,
        })
    }

    pub fn header_mut(&mut self) -> &mut CaptureHeader {
        &mut self.header
    }

    pub fn counts(&self) -> CaptureCounts {
        self.header.counts
    }

    /// Append a starting-state account.
    pub fn state(&mut self, update: CaptureUpdate) -> Result<(), CaptureError> {
        if self.seen_update {
            return Err(invalid(
                "starting state must come before the window's updates",
            ));
        }
        if !matches!(update.kind, CaptureUpdateKind::Account(_)) {
            return Err(invalid("starting state holds account updates only"));
        }
        self.header.counts.state += 1;
        self.write(&CaptureRecord::State { update })
    }

    /// Append an update in the window.
    pub fn update(&mut self, update: CaptureUpdate) -> Result<(), CaptureError> {
        self.seen_update = true;
        self.header.counts.updates += 1;
        match &update.kind {
            CaptureUpdateKind::Account(_) => self.header.counts.accounts += 1,
            CaptureUpdateKind::Transaction(_) => self.header.counts.transactions += 1,
        }
        self.write(&CaptureRecord::Update { update })
    }

    fn write(&mut self, record: &CaptureRecord) -> Result<(), CaptureError> {
        let slot = record.update().slot;
        let range = &mut self.header.slot_range;
        if self.header.counts.state + self.header.counts.updates == 1 {
            *range = SlotRange {
                first: slot,
                last: slot,
            };
        } else {
            range.first = range.first.min(slot);
            range.last = range.last.max(slot);
        }
        let mut line = serde_json::to_string(record).map_err(|error| invalid(error.to_string()))?;
        line.push('\n');
        self.hasher.update(line.as_bytes());
        self.body
            .as_mut()
            .ok_or_else(|| invalid("the capture is already finished"))?
            .write_all(line.as_bytes())?;
        Ok(())
    }

    /// Write the file and return its header.
    pub fn finish(mut self) -> Result<CaptureHeader, CaptureError> {
        let body = self
            .body
            .take()
            .ok_or_else(|| invalid("the capture is already finished"))?;
        body.into_inner()
            .map_err(|error| CaptureError::Io(error.into_error()))?
            .sync_all()?;
        let mut header = self.header.clone();
        header.content_hash = format!(
            "sha256:{}",
            hex_bytes::encode(&std::mem::take(&mut self.hasher).finalize())
        );
        {
            let file = File::create(&self.staging_path)?;
            let mut encoder = zstd::Encoder::new(file, ZSTD_LEVEL)?;
            let mut line = serde_json::to_string(&HeaderLine {
                kind: "header",
                header: &header,
            })
            .map_err(|error| invalid(error.to_string()))?;
            line.push('\n');
            encoder.write_all(line.as_bytes())?;
            std::io::copy(&mut File::open(&self.body_path)?, &mut encoder)?;
            encoder.finish()?.sync_all()?;
        }
        std::fs::rename(&self.staging_path, &self.path)?;
        Ok(header)
    }

    /// Drop the partial body without writing a capture.
    pub fn abandon(self) {}
}

impl Drop for CaptureWriter {
    fn drop(&mut self) {
        drop(self.body.take());
        let _ = std::fs::remove_file(&self.body_path);
        let _ = std::fs::remove_file(&self.staging_path);
    }
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// A capture read whole.
#[derive(Debug, Clone, PartialEq)]
pub struct Capture {
    pub header: CaptureHeader,
    pub records: Vec<CaptureRecord>,
}

impl Capture {
    pub fn updates(&self) -> impl Iterator<Item = &CaptureUpdate> {
        self.records.iter().map(CaptureRecord::update)
    }
}

/// Open a capture file for reading lines, decompressing it if it is zstd.
pub fn open_lines(path: &Path) -> Result<Box<dyn BufRead>, CaptureError> {
    let mut file = File::open(path)?;
    let mut magic = [0u8; 4];
    let read = file.read(&mut magic)?;
    let file = File::open(path)?;
    if read == 4 && magic == ZSTD_MAGIC {
        Ok(Box::new(BufReader::new(zstd::Decoder::new(file)?)))
    } else {
        Ok(Box::new(BufReader::new(file)))
    }
}

/// What a capture reader accepts, so that a corrupt or hostile file (a
/// small zstd frame that decompresses to gigabytes, a line with no end)
/// fails with an error instead of exhausting memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadLimits {
    /// The longest line, in decompressed bytes. An account's data is at most
    /// 10 MiB, 20 MiB as hex.
    pub max_line_bytes: usize,
    /// The most decompressed bytes in the whole file.
    pub max_total_bytes: u64,
    /// The most records after the header.
    pub max_records: u64,
}

impl Default for ReadLimits {
    fn default() -> Self {
        Self {
            max_line_bytes: 32 * 1024 * 1024,
            max_total_bytes: 4 * 1024 * 1024 * 1024,
            max_records: 20_000_000,
        }
    }
}

/// Lines of a reader, each at most `max_line_bytes` and together at most
/// `max_total_bytes`. A line ends at `\n`; a trailing `\r` is dropped, as
/// [`BufRead::lines`] does.
struct LimitedLines<R> {
    reader: R,
    limits: ReadLimits,
    total: u64,
    number: usize,
}

impl<R: BufRead> LimitedLines<R> {
    fn new(reader: R, limits: ReadLimits) -> Self {
        Self {
            reader,
            limits,
            total: 0,
            number: 0,
        }
    }

    fn next_line(&mut self) -> Result<Option<String>, CaptureError> {
        let mut line = Vec::new();
        loop {
            let available = self.reader.fill_buf()?;
            if available.is_empty() {
                if line.is_empty() {
                    return Ok(None);
                }
                break;
            }
            let (chunk, done) = match available.iter().position(|byte| *byte == b'\n') {
                Some(end) => (&available[..end], end + 1),
                None => (available, available.len()),
            };
            if line.len() + chunk.len() > self.limits.max_line_bytes {
                return Err(invalid(format!(
                    "line {} is longer than {} bytes",
                    self.number + 1,
                    self.limits.max_line_bytes
                )));
            }
            line.extend_from_slice(chunk);
            let ended = done > chunk.len();
            self.reader.consume(done);
            self.total += done as u64;
            if self.total > self.limits.max_total_bytes {
                return Err(invalid(format!(
                    "the capture decompresses to more than {} bytes",
                    self.limits.max_total_bytes
                )));
            }
            if ended {
                break;
            }
        }
        self.number += 1;
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        String::from_utf8(line)
            .map(Some)
            .map_err(|_| invalid(format!("line {} is not UTF-8", self.number)))
    }
}

/// Read and verify a capture: its schema, content hash and counts. A
/// headerless file of updates (the engine's raw capture) is read as legacy.
/// Every record is held in memory; [`verify_capture`] checks a file
/// without holding it.
pub fn read_capture(path: &Path) -> Result<Capture, CaptureError> {
    read_capture_with(path, ReadLimits::default())
}

/// [`read_capture`] with explicit limits.
pub fn read_capture_with(path: &Path, limits: ReadLimits) -> Result<Capture, CaptureError> {
    with_path(path, parse_capture_with(open_lines(path)?, limits))
}

/// Check a capture as [`read_capture`] does, one record at a time, and
/// return its header.
pub fn verify_capture(path: &Path, limits: ReadLimits) -> Result<CaptureHeader, CaptureError> {
    with_path(path, scan_capture(open_lines(path)?, limits, |_| Ok(())))
}

fn with_path<T>(path: &Path, result: Result<T, CaptureError>) -> Result<T, CaptureError> {
    result.map_err(|error| match error {
        CaptureError::Invalid(message) => invalid(format!("{}: {message}", path.display())),
        other => other,
    })
}

/// [`read_capture`] over any reader of (decompressed) lines.
pub fn parse_capture(reader: impl BufRead) -> Result<Capture, CaptureError> {
    parse_capture_with(reader, ReadLimits::default())
}

/// [`parse_capture`] with explicit limits.
pub fn parse_capture_with(
    reader: impl BufRead,
    limits: ReadLimits,
) -> Result<Capture, CaptureError> {
    let mut records = Vec::new();
    let header = scan_capture(reader, limits, |record| {
        records.push(record);
        Ok(())
    })?;
    Ok(Capture { header, records })
}

/// Read every record in order, handing each to `on_record`, and return the
/// header once the hash and counts have been checked (or, for a legacy
/// file, the header built from its records).
fn scan_capture(
    reader: impl BufRead,
    limits: ReadLimits,
    mut on_record: impl FnMut(CaptureRecord) -> Result<(), CaptureError>,
) -> Result<CaptureHeader, CaptureError> {
    let mut lines = LimitedLines::new(reader, limits);
    let first = loop {
        match lines.next_line()? {
            Some(line) if line.trim().is_empty() => continue,
            Some(line) => break line,
            None => return Err(invalid("the file is empty")),
        }
    };
    let first_value: Value = serde_json::from_str(&first)
        .map_err(|error| invalid(format!("line {}: {error}", lines.number)))?;
    if first_value.get("type").and_then(Value::as_str) != Some("header") {
        return scan_legacy(first_value, lines, on_record);
    }
    let HeaderLineOwned { header } =
        serde_json::from_value(first_value).map_err(|error| invalid(format!("header: {error}")))?;
    if header.schema != RAW_CAPTURE_SCHEMA {
        return Err(invalid(format!(
            "schema {} is not {RAW_CAPTURE_SCHEMA}",
            header.schema
        )));
    }
    if header.counts.state + header.counts.updates > limits.max_records {
        return Err(invalid(format!(
            "the header counts more than {} records",
            limits.max_records
        )));
    }
    let mut hasher = Sha256::new();
    let mut counts = CaptureCounts::default();
    let mut seen_update = false;
    while let Some(line) = lines.next_line()? {
        let line_number = lines.number;
        hasher.update(line.as_bytes());
        hasher.update(b"\n");
        if line.trim().is_empty() {
            continue;
        }
        if counts.state + counts.updates >= limits.max_records {
            return Err(invalid(format!(
                "the file holds more than {} records",
                limits.max_records
            )));
        }
        let record: CaptureRecord = serde_json::from_str(&line)
            .map_err(|error| invalid(format!("line {line_number}: {error}")))?;
        match &record {
            CaptureRecord::State { update } => {
                if seen_update {
                    return Err(invalid(format!(
                        "line {line_number}: starting state after the window's updates"
                    )));
                }
                if !matches!(update.kind, CaptureUpdateKind::Account(_)) {
                    return Err(invalid(format!(
                        "line {line_number}: starting state must be an account"
                    )));
                }
                counts.state += 1;
            }
            CaptureRecord::Update { update } => {
                seen_update = true;
                counts.updates += 1;
                match update.kind {
                    CaptureUpdateKind::Account(_) => counts.accounts += 1,
                    CaptureUpdateKind::Transaction(_) => counts.transactions += 1,
                }
            }
        }
        on_record(record)?;
    }
    let hash = format!("sha256:{}", hex_bytes::encode(&hasher.finalize()));
    if hash != header.content_hash {
        return Err(invalid(format!(
            "content hash {hash} does not match the header's {}",
            header.content_hash
        )));
    }
    if counts != header.counts {
        return Err(invalid(format!(
            "the header counts {:?} but the file holds {:?}",
            header.counts, counts
        )));
    }
    Ok(header)
}

fn scan_legacy(
    first: Value,
    mut lines: LimitedLines<impl BufRead>,
    mut on_record: impl FnMut(CaptureRecord) -> Result<(), CaptureError>,
) -> Result<CaptureHeader, CaptureError> {
    let mut programs: Vec<String> = Vec::new();
    let mut header = CaptureHeader::legacy(Vec::new());
    let mut slots: Option<SlotRange> = None;
    let limits = lines.limits;
    let mut take = |value: Value, line_number: usize| -> Result<(), CaptureError> {
        if header.counts.updates >= limits.max_records {
            return Err(invalid(format!(
                "the file holds more than {} records",
                limits.max_records
            )));
        }
        let update: CaptureUpdate = serde_json::from_value(value)
            .map_err(|error| invalid(format!("line {line_number}: {error}")))?;
        header.counts.updates += 1;
        match &update.kind {
            CaptureUpdateKind::Account(account) => {
                header.counts.accounts += 1;
                if !programs.contains(&account.owner) {
                    programs.push(account.owner.clone());
                }
            }
            CaptureUpdateKind::Transaction(transaction) => {
                header.counts.transactions += 1;
                for instruction in &transaction.instructions {
                    if !programs.contains(&instruction.program_id) {
                        programs.push(instruction.program_id.clone());
                    }
                }
            }
        }
        slots = Some(match slots {
            None => SlotRange {
                first: update.slot,
                last: update.slot,
            },
            Some(range) => SlotRange {
                first: range.first.min(update.slot),
                last: range.last.max(update.slot),
            },
        });
        on_record(CaptureRecord::Update { update })
    };
    take(first, lines.number)?;
    while let Some(line) = lines.next_line()? {
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str(&line)
            .map_err(|error| invalid(format!("line {}: {error}", lines.number)))?;
        take(value, lines.number)?;
    }
    if let Some(range) = slots {
        header.slot_range = range;
    }
    header.programs = programs;
    Ok(header)
}

/// Read only the header (the first line). Cheap: nothing else is
/// decompressed, except for a legacy file, which has no header and is
/// scanned whole.
pub fn read_header(path: &Path) -> Result<CaptureHeader, CaptureError> {
    let mut lines = LimitedLines::new(open_lines(path)?, ReadLimits::default());
    let first = lines
        .next_line()
        .map_err(|error| match error {
            CaptureError::Invalid(message) => invalid(format!("{}: {message}", path.display())),
            other => other,
        })?
        .ok_or_else(|| invalid(format!("{} is empty", path.display())))?;
    let value: Value = serde_json::from_str(&first)
        .map_err(|error| invalid(format!("{}: {error}", path.display())))?;
    if value.get("type").and_then(Value::as_str) != Some("header") {
        return verify_capture(path, ReadLimits::default());
    }
    let HeaderLineOwned { header, .. } = serde_json::from_value(value)
        .map_err(|error| invalid(format!("{}: header: {error}", path.display())))?;
    Ok(header)
}

/// Lowercase hex for byte fields.
pub mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    const DIGITS: &[u8; 16] = b"0123456789abcdef";

    pub fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            out.push(DIGITS[(byte >> 4) as usize] as char);
            out.push(DIGITS[(byte & 0x0f) as usize] as char);
        }
        out
    }

    pub fn decode(text: &str) -> Result<Vec<u8>, String> {
        fn nibble(c: u8) -> Result<u8, String> {
            match c {
                b'0'..=b'9' => Ok(c - b'0'),
                b'a'..=b'f' => Ok(c - b'a' + 10),
                b'A'..=b'F' => Ok(c - b'A' + 10),
                _ => Err(format!("invalid hex digit {:?}", c as char)),
            }
        }
        let bytes = text.as_bytes();
        if !bytes.len().is_multiple_of(2) {
            return Err("odd-length hex".into());
        }
        bytes
            .chunks(2)
            .map(|pair| Ok(nibble(pair[0])? << 4 | nibble(pair[1])?))
            .collect()
    }

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(deserializer)?;
        decode(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn producer() -> CaptureProducer {
        CaptureProducer {
            tool: "a4".into(),
            version: "0.0.0-test".into(),
            method: "rpc".into(),
            source: Some("rpc.example".into()),
            created_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    fn account(seq: u64, slot: u64, pubkey: &str) -> CaptureUpdate {
        CaptureUpdate {
            seq,
            slot,
            received_at: 1_700_000_000_000 + slot as i64,
            kind: CaptureUpdateKind::Account(CaptureAccount {
                pubkey: pubkey.into(),
                owner: "Prog1111111111111111111111111111111111111111".into(),
                data: vec![0, 1, 2, 0xff],
                lamports: 42,
                write_version: seq,
                txn_signature: None,
            }),
        }
    }

    fn transaction(seq: u64, slot: u64) -> CaptureUpdate {
        CaptureUpdate {
            seq,
            slot,
            received_at: 1_700_000_000_000,
            kind: CaptureUpdateKind::Transaction(CaptureTransaction {
                account_flags: CaptureAccountFlags {
                    signers: vec!["Payer".into()],
                    writable: vec!["Payer".into()],
                },
                transaction_metadata: Some(serde_json::json!({ "version": 0 })),
                signature: format!("sig{seq}"),
                error: None,
                txn_index: 3,
                top_level_instructions: vec![CaptureTopLevelInstruction {
                    program_id: "Prog".into(),
                    data: vec![9],
                    accounts: vec!["A".into()],
                }],
                log_messages: vec!["Program Prog invoke [1]".into()],
                instructions: vec![CaptureInstruction {
                    program_id: "Prog".into(),
                    data: vec![9],
                    accounts: vec!["A".into()],
                    is_inner: false,
                    top_level_index: Some(0),
                    parent_index: None,
                    ix_path: Some("0".into()),
                }],
            }),
        }
    }

    #[test]
    fn a_capture_round_trips_with_its_hash_and_counts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("window.arcap");
        let mut writer = CaptureWriter::create(
            &path,
            CaptureHeader::new(vec!["Prog".into()], "mainnet-beta", producer()),
        )
        .unwrap();
        writer.state(account(0, 100, "A")).unwrap();
        writer.update(transaction(1, 101)).unwrap();
        writer.update(account(2, 102, "A")).unwrap();
        let written = writer.finish().unwrap();
        assert!(written.content_hash.starts_with("sha256:"));
        assert_eq!(
            written.counts,
            CaptureCounts {
                state: 1,
                updates: 2,
                transactions: 1,
                accounts: 1
            }
        );
        assert_eq!(
            written.slot_range,
            SlotRange {
                first: 100,
                last: 102
            }
        );
        // Compressed on disk, and nothing left behind.
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(&bytes[..4], &ZSTD_MAGIC);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);

        let capture = read_capture(&path).unwrap();
        assert_eq!(capture.header, written);
        assert_eq!(read_header(&path).unwrap(), written);
        assert_eq!(capture.records.len(), 3);
        assert!(capture.records[0].is_state());
        assert_eq!(capture.records[1].update(), &transaction(1, 101));
        assert_eq!(capture.records[2].update(), &account(2, 102, "A"));
    }

    #[test]
    fn a_tampered_capture_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("window.arcap");
        let mut writer = CaptureWriter::create(
            &path,
            CaptureHeader::new(vec!["Prog".into()], "devnet", producer()),
        )
        .unwrap();
        writer.update(account(0, 7, "A")).unwrap();
        writer.finish().unwrap();
        let mut text = String::new();
        open_lines(&path)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        let tampered = text.replace("\"lamports\":42", "\"lamports\":43");
        assert_ne!(text, tampered);
        let error = parse_capture(std::io::Cursor::new(tampered)).unwrap_err();
        assert!(error.to_string().contains("content hash"), "{error}");
    }

    fn written(dir: &Path, updates: u64) -> PathBuf {
        let path = dir.join("window.arcap");
        let mut writer = CaptureWriter::create(
            &path,
            CaptureHeader::new(vec!["Prog".into()], "devnet", producer()),
        )
        .unwrap();
        for seq in 0..updates {
            writer.update(account(seq, 7 + seq, "A")).unwrap();
        }
        writer.finish().unwrap();
        path
    }

    #[test]
    fn truncated_and_corrupt_captures_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = written(dir.path(), 50);
        let bytes = std::fs::read(&path).unwrap();

        let truncated = dir.path().join("truncated.arcap");
        std::fs::write(&truncated, &bytes[..bytes.len() * 2 / 3]).unwrap();
        assert!(read_capture(&truncated).is_err());
        assert!(verify_capture(&truncated, ReadLimits::default()).is_err());

        let mut flipped = bytes.clone();
        let middle = flipped.len() / 2;
        flipped[middle] ^= 0xff;
        let corrupt = dir.path().join("corrupt.arcap");
        std::fs::write(&corrupt, &flipped).unwrap();
        assert!(read_capture(&corrupt).is_err());

        let garbage = dir.path().join("garbage.arcap");
        std::fs::write(&garbage, [0x28, 0xb5, 0x2f, 0xfd, 1, 2, 3]).unwrap();
        assert!(read_capture(&garbage).is_err());

        assert_eq!(
            verify_capture(&path, ReadLimits::default()).unwrap(),
            read_capture(&path).unwrap().header
        );
    }

    #[test]
    fn oversized_captures_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = written(dir.path(), 20);
        let limits = ReadLimits::default();
        let error = read_capture_with(
            &path,
            ReadLimits {
                max_line_bytes: 64,
                ..limits
            },
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("longer than 64 bytes"),
            "{error}"
        );
        let error = read_capture_with(
            &path,
            ReadLimits {
                max_total_bytes: 2_000,
                ..limits
            },
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("decompresses to more"),
            "{error}"
        );
        let error = verify_capture(
            &path,
            ReadLimits {
                max_records: 5,
                ..limits
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("more than 5 records"), "{error}");

        // A line with no end stops at the limit rather than filling memory.
        let endless = std::io::Cursor::new(vec![b'x'; 10_000]);
        let error = parse_capture_with(
            endless,
            ReadLimits {
                max_line_bytes: 1_000,
                ..limits
            },
        )
        .unwrap_err();
        assert!(error.to_string().contains("longer than"), "{error}");
    }

    #[test]
    fn an_unfinished_writer_leaves_no_files() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut writer = CaptureWriter::create(
                dir.path().join("x.arcap"),
                CaptureHeader::new(vec![], "devnet", producer()),
            )
            .unwrap();
            writer.update(account(0, 7, "A")).unwrap();
            // Dropped: an interrupted capture.
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn state_after_updates_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut writer = CaptureWriter::create(
            dir.path().join("x.arcap"),
            CaptureHeader::new(vec![], "devnet", producer()),
        )
        .unwrap();
        writer.update(account(0, 7, "A")).unwrap();
        assert!(writer.state(account(1, 7, "B")).is_err());
        assert!(writer.state(transaction(1, 7)).is_err());
        writer.abandon();
    }

    #[test]
    fn a_headerless_engine_capture_reads_as_legacy() {
        // The JSON lines a runtime's raw capture writes: one update per line.
        let lines = [
            serde_json::to_string(&account(0, 5, "A")).unwrap(),
            serde_json::to_string(&transaction(1, 6)).unwrap(),
        ]
        .join("\n");
        let capture = parse_capture(std::io::Cursor::new(lines)).unwrap();
        assert!(capture.header.is_legacy());
        assert_eq!(capture.header.counts.updates, 2);
        assert_eq!(capture.header.slot_range, SlotRange { first: 5, last: 6 });
        assert_eq!(
            capture.header.programs,
            vec!["Prog1111111111111111111111111111111111111111", "Prog"]
        );
    }

    #[test]
    fn updates_serialize_in_the_runtime_shape() {
        let value = serde_json::to_value(transaction(1, 101)).unwrap();
        assert_eq!(value["kind"]["Transaction"]["signature"], "sig1");
        assert_eq!(
            value["kind"]["Transaction"]["instructions"][0]["data"],
            "09"
        );
        assert_eq!(value["received_at"], 1_700_000_000_000i64);
        let value = serde_json::to_value(account(0, 1, "A")).unwrap();
        assert_eq!(value["kind"]["Account"]["data"], "000102ff");
        assert!(value["kind"]["Account"].get("txn_signature").is_none());
    }

    #[test]
    fn account_flags_follow_the_message_header() {
        let keys: Vec<String> = ["a", "b", "c", "d", "e"].map(String::from).to_vec();
        // 2 signers (1 read-only), 1 read-only unsigned.
        let flags = CaptureAccountFlags::from_message(2, 1, 1, &keys, &["z".into()]);
        assert_eq!(flags.signers, vec!["a", "b"]);
        assert_eq!(flags.writable, vec!["a", "c", "d", "z"]);
    }
}
