//! Optional telemetry initialization helper.
//!
//! Provides a convenient way to initialize tracing with optional OpenTelemetry integration.
//! This is an optional helper - you can configure tracing yourself if you prefer.

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

#[cfg(feature = "otel")]
use opentelemetry::KeyValue;
#[cfg(feature = "otel")]
use opentelemetry_sdk::trace::Sampler;
#[cfg(feature = "otel")]
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct TelemetryConfig {
    pub service_name: String,
    pub json_logs: bool,
    #[cfg(feature = "otel")]
    pub otlp_endpoint: Option<String>,
    #[cfg(feature = "otel")]
    pub resource_attributes: Vec<KeyValue>,
    #[cfg(feature = "otel")]
    pub metrics_period: Duration,
    #[cfg(feature = "otel")]
    pub trace_sampler: Sampler,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            service_name: "arete".to_string(),
            json_logs: false,
            #[cfg(feature = "otel")]
            otlp_endpoint: None,
            #[cfg(feature = "otel")]
            resource_attributes: Vec::new(),
            #[cfg(feature = "otel")]
            metrics_period: Duration::from_secs(60),
            #[cfg(feature = "otel")]
            trace_sampler: Sampler::ParentBased(Box::new(Sampler::AlwaysOn)),
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

    /// Add resource attributes shared by every signal emitted by this process.
    #[cfg(feature = "otel")]
    pub fn with_resource_attributes(mut self, attributes: Vec<KeyValue>) -> Self {
        self.resource_attributes.extend(attributes);
        self
    }

    /// Set the standard OpenTelemetry service version resource attribute.
    #[cfg(feature = "otel")]
    pub fn with_service_version(mut self, version: impl Into<String>) -> Self {
        self.resource_attributes
            .push(KeyValue::new("service.version", version.into()));
        self
    }

    /// Set the standard OpenTelemetry deployment environment resource attribute.
    #[cfg(feature = "otel")]
    pub fn with_deployment_environment(mut self, environment: impl Into<String>) -> Self {
        self.resource_attributes
            .push(KeyValue::new("deployment.environment", environment.into()));
        self
    }

    /// Set the interval at which metrics are exported. Defaults to 60 seconds.
    #[cfg(feature = "otel")]
    pub fn with_metrics_period(mut self, period: Duration) -> Self {
        self.metrics_period = period;
        self
    }

    /// Set the SDK's head sampler.
    ///
    /// The default is parent-based always-on sampling, allowing a collector to
    /// make the final tail-sampling decision.
    #[cfg(feature = "otel")]
    pub fn with_trace_sampler(mut self, sampler: Sampler) -> Self {
        self.trace_sampler = sampler;
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

    anyhow::ensure!(
        !config.metrics_period.is_zero(),
        "OpenTelemetry metrics export period must be greater than zero"
    );
    global::set_text_map_propagator(TraceContextPropagator::new());

    let endpoint = config
        .otlp_endpoint
        .as_deref()
        .unwrap_or("http://localhost:4317");

    let resource = telemetry_resource(&config);

    let tracer: Tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint(endpoint),
        )
        .with_trace_config(
            opentelemetry_sdk::trace::config()
                .with_sampler(config.trace_sampler)
                .with_resource(resource.clone()),
        )
        .install_batch(opentelemetry_sdk::runtime::Tokio)?;

    let meter_provider = opentelemetry_otlp::new_pipeline()
        .metrics(opentelemetry_sdk::runtime::Tokio)
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint(endpoint),
        )
        .with_resource(resource)
        .with_period(config.metrics_period)
        .build()?;
    global::set_meter_provider(meter_provider.clone());

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

    Ok(TelemetryGuard { meter_provider })
}

#[cfg(feature = "otel")]
fn telemetry_resource(config: &TelemetryConfig) -> opentelemetry_sdk::Resource {
    let mut attributes = vec![opentelemetry::KeyValue::new(
        "service.name",
        config.service_name.clone(),
    )];
    attributes.extend(config.resource_attributes.iter().cloned());
    opentelemetry_sdk::Resource::new(attributes)
}

#[cfg(feature = "otel")]
pub struct TelemetryGuard {
    meter_provider: opentelemetry_sdk::metrics::SdkMeterProvider,
}

#[cfg(feature = "otel")]
impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if let Err(error) = self.meter_provider.shutdown() {
            tracing::warn!(%error, "failed to shut down OpenTelemetry meter provider");
        }
        opentelemetry::global::shutdown_tracer_provider();
    }
}

#[cfg(all(test, feature = "otel"))]
mod tests {
    use super::*;
    use opentelemetry::Key;

    #[test]
    fn telemetry_resource_contains_service_and_configured_attributes() {
        let config = TelemetryConfig::new("test-service")
            .with_service_version("1.2.3")
            .with_deployment_environment("test")
            .with_resource_attributes(vec![KeyValue::new("custom.key", "custom-value")]);

        let resource = telemetry_resource(&config);

        assert_eq!(
            resource.get(Key::new("service.name")),
            Some("test-service".into())
        );
        assert_eq!(
            resource.get(Key::new("service.version")),
            Some("1.2.3".into())
        );
        assert_eq!(
            resource.get(Key::new("deployment.environment")),
            Some("test".into())
        );
        assert_eq!(
            resource.get(Key::new("custom.key")),
            Some("custom-value".into())
        );
    }

    #[test]
    fn zero_metrics_period_is_rejected() {
        let error = match init_with_otel(
            TelemetryConfig::new("test-service").with_metrics_period(Duration::ZERO),
        ) {
            Ok(_) => panic!("a zero export period must not reach Tokio's interval"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("must be greater than zero"));
    }
}
