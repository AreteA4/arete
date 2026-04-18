//! Optional telemetry initialization helper.
//!
//! Provides a convenient way to initialize tracing with optional OpenTelemetry integration.
//! This is an optional helper - you can configure tracing yourself if you prefer.

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    pub service_name: String,
    pub json_logs: bool,
    #[cfg(feature = "otel")]
    pub otlp_endpoint: Option<String>,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            service_name: "arete".to_string(),
            json_logs: false,
            #[cfg(feature = "otel")]
            otlp_endpoint: None,
        }
    }
}

impl TelemetryConfig {
    pub fn new(service_name: impl Into<String>) -> Self {
        Self {
            service_name: service_name.into(),
            ..Default::default()
        }
    }

    pub fn with_json_logs(mut self, enabled: bool) -> Self {
        self.json_logs = enabled;
        self
    }

    #[cfg(feature = "otel")]
    pub fn with_otlp_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.otlp_endpoint = Some(endpoint.into());
        self
    }
}

pub fn init(config: TelemetryConfig) -> anyhow::Result<()> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let registry = tracing_subscriber::registry().with(env_filter);

    if config.json_logs {
        let fmt_layer = tracing_subscriber::fmt::layer().json().flatten_event(true);
        registry.with(fmt_layer).init();
    } else {
        let fmt_layer = tracing_subscriber::fmt::layer();
        registry.with(fmt_layer).init();
    }

    Ok(())
}

#[cfg(feature = "otel")]
pub fn init_with_otel(config: TelemetryConfig) -> anyhow::Result<TelemetryGuard> {
    use opentelemetry::global;
    use opentelemetry_otlp::WithExportConfig;
    use opentelemetry_sdk::propagation::TraceContextPropagator;
    use opentelemetry_sdk::trace::Tracer;
    use opentelemetry_sdk::Resource;

    global::set_text_map_propagator(TraceContextPropagator::new());

    let endpoint = config
        .otlp_endpoint
        .as_deref()
        .unwrap_or("http://localhost:4317");

    let tracer: Tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint(endpoint),
        )
        .with_trace_config(
            opentelemetry_sdk::trace::config().with_resource(Resource::new(vec![
                opentelemetry::KeyValue::new("service.name", config.service_name.clone()),
            ])),
        )
        .install_batch(opentelemetry_sdk::runtime::Tokio)?;

    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(otel_layer);

    if config.json_logs {
        let fmt_layer = tracing_subscriber::fmt::layer().json().flatten_event(true);
        registry.with(fmt_layer).init();
    } else {
        let fmt_layer = tracing_subscriber::fmt::layer();
        registry.with(fmt_layer).init();
    }

    Ok(TelemetryGuard)
}

#[cfg(feature = "otel")]
pub struct TelemetryGuard;

#[cfg(feature = "otel")]
impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        opentelemetry::global::shutdown_tracer_provider();
    }
}
