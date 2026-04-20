//! Canonical Structured Logging
//!
//! Accumulates context throughout event processing, emits ONE log line at the end.
//!
//! When the `otel` feature is enabled, trace context (trace_id, span_id) is automatically
//! included in emitted logs for correlation with distributed traces.

use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Instant;

#[cfg(feature = "otel")]
use opentelemetry::trace::TraceContextExt;
#[cfg(feature = "otel")]
use tracing_opentelemetry::OpenTelemetrySpanExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

pub struct CanonicalLog {
    data: HashMap<String, Value>,
    start: Instant,
    level: LogLevel,
    emitted: bool,
}

impl CanonicalLog {
    pub fn new() -> Self {
        Self {
            data: HashMap::new(),
            start: Instant::now(),
            level: LogLevel::Info,
            emitted: false,
        }
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Serialize) -> &mut Self {
        if let Ok(v) = serde_json::to_value(value) {
            self.data.insert(key.into(), v);
        }
        self
    }

    pub fn set_level(&mut self, level: LogLevel) -> &mut Self {
        self.level = level;
        self
    }

    pub fn inc(&mut self, key: &str, amount: i64) -> &mut Self {
        let current = self.data.get(key).and_then(|v| v.as_i64()).unwrap_or(0);
        self.data.insert(key.to_string(), json!(current + amount));
        self
    }

    pub fn duration_ms(&self) -> f64 {
        self.start.elapsed().as_secs_f64() * 1000.0
    }

    pub fn suppress(&mut self) {
        self.emitted = true;
    }

    pub fn emit(mut self) {
        self.do_emit();
    }

    fn do_emit(&mut self) {
        if self.emitted {
            return;
        }
        self.emitted = true;

        self.data
            .insert("duration_ms".to_string(), json!(self.duration_ms()));

        #[cfg(feature = "otel")]
        {
            let span = tracing::Span::current();
            let context = span.context();
            let span_ref = context.span();
            let span_context = span_ref.span_context();
            if span_context.is_valid() {
                self.data.insert(
                    "trace_id".to_string(),
                    json!(format!("{:032x}", span_context.trace_id())),
                );
                self.data.insert(
                    "span_id".to_string(),
                    json!(format!("{:016x}", span_context.span_id())),
                );
            }
        }

        // Emit as a structured field so OTEL/Axiom can parse it, rather than embedding JSON in message body
        let canonical = serde_json::to_string(&self.data).unwrap_or_else(|_| "{}".to_string());

        match self.level {
            LogLevel::Trace => {
                tracing::trace!(target: "arete::canonical", canonical = %canonical, "canonical_event")
            }
            LogLevel::Debug => {
                tracing::debug!(target: "arete::canonical", canonical = %canonical, "canonical_event")
            }
            LogLevel::Info => {
                tracing::info!(target: "arete::canonical", canonical = %canonical, "canonical_event")
            }
            LogLevel::Warn => {
                tracing::warn!(target: "arete::canonical", canonical = %canonical, "canonical_event")
            }
            LogLevel::Error => {
                tracing::error!(target: "arete::canonical", canonical = %canonical, "canonical_event")
            }
        }
    }
}

impl Default for CanonicalLog {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for CanonicalLog {
    fn drop(&mut self) {
        self.do_emit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_usage() {
        let mut log = CanonicalLog::new();
        log.set("event_type", "BuyIxState")
            .set("slot", 12345)
            .set("mutations", 3);
        log.suppress();
        assert!(log.data.contains_key("event_type"));
    }

    #[test]
    fn test_increment() {
        let mut log = CanonicalLog::new();
        log.inc("cache_hits", 1);
        log.inc("cache_hits", 1);
        log.inc("cache_hits", 1);
        log.suppress();
        assert_eq!(log.data.get("cache_hits"), Some(&json!(3)));
    }
}
