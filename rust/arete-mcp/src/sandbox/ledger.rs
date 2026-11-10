//! The sandbox ledger: every action the sandbox wallet ran (history) and
//! what it holds now (holdings), as the Arete API records them.
//!
//! History pages are newest first:
//! `{ items: [{ id, createdAt, status, signature, slot, programs[],
//! instructions[], events[{ program, name, path, source, fields, data }],
//! solDelta, tokenDeltas[{ mint, delta, decimals }], accounts[], raw }],
//! nextCursor }`, where `raw` is the parsed transaction with its pre and
//! post balances. `events` are the Anchor events the action emitted, in
//! instruction-path order; one that did not decode has
//! `fields.__parse_error` and keeps its bytes in `data` (base64), with
//! `name: null` when no IDL event matched it. Holdings:
//! `{ sol, tokens[{ mint, amount, decimals, acquiredBySandbox }],
//! positions[{ program, account, label, raw }], asOfSlot }`.
//!
//! The API pages by `cursor` and `limit`. The filters here (`status`,
//! `program`, `mint`, `since`) are applied by this client to the pages it
//! reads, reading further pages until enough items match.

use chrono::{DateTime, Utc};
use serde_json::{json, Value};

/// Items per page when none is asked for.
pub const DEFAULT_LIMIT: u32 = 20;
/// The most items one page may ask for.
pub const MAX_LIMIT: u32 = 100;
/// Pages one filtered read may scan before it returns what it found and the
/// cursor to carry on from.
pub const MAX_SCAN_PAGES: usize = 10;

/// One history read: a page position and the filters to apply.
#[derive(Debug, Clone, Default)]
pub struct HistoryQuery {
    pub cursor: Option<String>,
    pub limit: Option<u32>,
    /// Only actions with this status (`landed`, `failed`, `rejected`, ...).
    pub status: Option<String>,
    /// Only actions that called this program (its id or its name).
    pub program: Option<String>,
    /// Only actions that moved this token mint.
    pub mint: Option<String>,
    /// Only actions created at or after this time.
    pub since: Option<DateTime<Utc>>,
}

impl HistoryQuery {
    pub fn limit(&self) -> u32 {
        self.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
    }

    pub fn filtered(&self) -> bool {
        self.status.is_some()
            || self.program.is_some()
            || self.mint.is_some()
            || self.since.is_some()
    }

    /// The query string for one page of `page_limit` items (at most
    /// [`MAX_LIMIT`]), from `cursor`.
    pub(crate) fn page_query(&self, cursor: Option<&str>, page_limit: u32) -> String {
        let mut query = format!("?limit={}", page_limit.clamp(1, MAX_LIMIT));
        if let Some(cursor) = cursor {
            query.push_str("&cursor=");
            query.push_str(&super::url_encode(cursor));
        }
        query
    }

    /// Whether `item` passes every filter.
    pub fn matches(&self, item: &Value) -> bool {
        if let Some(status) = &self.status {
            if !item["status"]
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(status))
            {
                return false;
            }
        }
        if let Some(program) = &self.program {
            if !item_programs(item)
                .chain(instruction_programs(item))
                .any(|value| value.eq_ignore_ascii_case(program))
            {
                return false;
            }
        }
        if let Some(mint) = &self.mint {
            let moved = item["tokenDeltas"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|delta| delta["mint"].as_str() == Some(mint.as_str()));
            if !moved {
                return false;
            }
        }
        if let Some(since) = &self.since {
            // An action whose time cannot be read is kept, not hidden.
            if created_at(item).is_some_and(|created| created < *since) {
                return false;
            }
        }
        true
    }
}

pub fn created_at(item: &Value) -> Option<DateTime<Utc>> {
    item["createdAt"]
        .as_str()
        .and_then(|text| DateTime::parse_from_rfc3339(text).ok())
        .map(|time| time.with_timezone(&Utc))
}

/// The ids and names of the programs an item called: `programs[]` entries
/// are program ids, or objects with `programId` and `name`.
pub fn item_programs(item: &Value) -> impl Iterator<Item = &str> {
    item["programs"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|program| match program {
            Value::String(id) => vec![id.as_str()],
            Value::Object(object) => ["programId", "id", "name", "label"]
                .iter()
                .filter_map(|key| object.get(*key).and_then(Value::as_str))
                .collect(),
            _ => Vec::new(),
        })
}

/// The programs an item's decoded instructions name (`program`,
/// `programName`): an action can carry a program's name only there.
pub fn instruction_programs(item: &Value) -> impl Iterator<Item = &str> {
    item["instructions"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|instruction| {
            ["program", "programId", "programName"]
                .iter()
                .filter_map(|key| instruction.get(*key).and_then(Value::as_str))
                .collect::<Vec<_>>()
        })
}

/// A page the scan assembled: the matching items and where to carry on.
pub(crate) struct Scan {
    pub items: Vec<Value>,
    pub next_cursor: Option<String>,
    pub pages_read: usize,
}

impl Scan {
    pub(crate) fn into_json(self, query: &HistoryQuery) -> Value {
        let mut body = json!({
            "items": self.items,
            "nextCursor": self.next_cursor,
        });
        if query.filtered() {
            body["pagesRead"] = json!(self.pages_read);
        }
        body
    }
}

/// `raw` taken out of every item, for answers that must stay small.
pub fn without_raw(mut body: Value) -> Value {
    if let Some(items) = body.get_mut("items").and_then(Value::as_array_mut) {
        for item in items {
            if let Some(object) = item.as_object_mut() {
                object.remove("raw");
            }
        }
    }
    if let Some(positions) = body.get_mut("positions").and_then(Value::as_array_mut) {
        for position in positions {
            if let Some(object) = position.as_object_mut() {
                object.remove("raw");
            }
        }
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> Value {
        json!({
            "id": "a1", "createdAt": "2026-11-10T12:00:00Z", "status": "landed",
            "programs": ["6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P",
                         { "programId": "Sys1", "name": "System" }],
            "tokenDeltas": [{ "mint": "M1", "delta": "5", "decimals": 6 }],
            "raw": { "big": true }
        })
    }

    #[test]
    fn filters_match_status_program_mint_and_time() {
        let item = item();
        let all = HistoryQuery::default();
        assert!(all.matches(&item));
        let query = |f: fn(&mut HistoryQuery)| {
            let mut query = HistoryQuery::default();
            f(&mut query);
            query
        };
        assert!(query(|q| q.status = Some("LANDED".into())).matches(&item));
        assert!(!query(|q| q.status = Some("failed".into())).matches(&item));
        assert!(query(|q| q.program = Some("system".into())).matches(&item));
        assert!(
            query(|q| q.program = Some("6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P".into()))
                .matches(&item)
        );
        assert!(!query(|q| q.program = Some("Kamino".into())).matches(&item));
        // A name only the decoded instruction carries still matches.
        let by_instruction = json!({
            "status": "rejected",
            "programs": ["6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P"],
            "instructions": [{ "programName": "Pump.fun", "name": "sell" }]
        });
        assert!(query(|q| q.program = Some("pump.fun".into())).matches(&by_instruction));
        assert!(query(|q| q.mint = Some("M1".into())).matches(&item));
        assert!(!query(|q| q.mint = Some("M2".into())).matches(&item));
        let since = |text: &str| HistoryQuery {
            since: Some(DateTime::parse_from_rfc3339(text).unwrap().into()),
            ..Default::default()
        };
        assert!(since("2026-11-10T00:00:00Z").matches(&item));
        assert!(!since("2026-11-11T00:00:00Z").matches(&item));
        // No readable time: kept.
        assert!(since("2026-11-11T00:00:00Z").matches(&json!({ "status": "landed" })));
    }

    #[test]
    fn limits_are_clamped_and_cursors_encoded() {
        let mut query = HistoryQuery::default();
        assert_eq!(query.page_query(None, query.limit()), "?limit=20");
        query.limit = Some(500);
        assert_eq!(
            query.page_query(Some("a b/c"), query.limit()),
            "?limit=100&cursor=a%20b%2Fc"
        );
        query.limit = Some(0);
        assert_eq!(query.limit(), 1);
    }

    #[test]
    fn raw_is_dropped_from_items_and_positions() {
        let body = without_raw(json!({
            "items": [item()],
            "positions": [{ "account": "A", "raw": {} }],
            "nextCursor": "c"
        }));
        assert!(body["items"][0].get("raw").is_none());
        assert!(body["positions"][0].get("raw").is_none());
        assert_eq!(body["items"][0]["id"], "a1");
        assert_eq!(body["nextCursor"], "c");
    }
}
