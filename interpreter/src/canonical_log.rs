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
use opentelemetry::{trace::TraceContextExt, KeyValue};
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

        // Most events are logged at a level the subscriber filters out. Find
        // that out before building anything: the line, the trace ids and the
        // span event all cost more than the event's own processing.
        let log_enabled = level_enabled(self.level);
        #[cfg(feature = "otel")]
        let span_events = canonical_span_events_enabled();
        #[cfg(not(feature = "otel"))]
        let span_events = false;
        if !log_enabled && !span_events {
            return;
        }

        self.data
            .insert("duration_ms".to_string(), json!(self.duration_ms()));

        #[cfg(feature = "otel")]
        {
            let span = tracing::Span::current();
            let context = span.context();
            let span_ref = context.span();
            let span_context = span_ref.span_context();
            if span_context.is_valid() {
                if span_events {
                    span_ref.add_event(
                        "canonical_event",
                        canonical_span_event_attributes(&self.data),
                    );
                }
                if log_enabled {
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
        }
        if !log_enabled {
            return;
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

/// Whether the subscriber would record a canonical line at `level`.
fn level_enabled(level: LogLevel) -> bool {
    match level {
        LogLevel::Trace => tracing::enabled!(target: "arete::canonical", tracing::Level::TRACE),
        LogLevel::Debug => tracing::enabled!(target: "arete::canonical", tracing::Level::DEBUG),
        LogLevel::Info => tracing::enabled!(target: "arete::canonical", tracing::Level::INFO),
        LogLevel::Warn => tracing::enabled!(target: "arete::canonical", tracing::Level::WARN),
        LogLevel::Error => tracing::enabled!(target: "arete::canonical", tracing::Level::ERROR),
    }
}

/// `ARETE_CANONICAL_SPAN_EVENTS`, read once: it is consulted for every event.
#[cfg(feature = "otel")]
fn canonical_span_events_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("ARETE_CANONICAL_SPAN_EVENTS")
            .map(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false)
    })
}

#[cfg(feature = "otel")]
fn canonical_span_event_attributes(data: &HashMap<String, Value>) -> Vec<KeyValue> {
    const FIELDS: [&str; 10] = [
        "phase",
        "event_type",
        "entity",
        "outcome",
        "duration_ms",
        "opcodes",
        "mutations",
        "pda_hits",
        "pda_misses",
        "skip_reason",
    ];

    FIELDS
        .into_iter()
        .filter_map(|key| {
            let value = data.get(key)?;
            let value = match value {
                Value::String(value) => value.clone().into(),
                Value::Bool(value) => (*value).into(),
                Value::Number(value) => {
                    if let Some(value) = value.as_i64() {
                        value.into()
                    } else if let Some(value) =
                        value.as_u64().and_then(|value| value.try_into().ok())
                    {
                        opentelemetry::Value::I64(value)
                    } else {
                        value.as_f64()?.into()
                    }
                }
                _ => return None,
            };
            Some(KeyValue::new(key, value))
        })
        .collect()
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
