//! OTLP/HTTP JSON, using integer enums and decimal uint64 fields as required by
//! <https://opentelemetry.io/docs/specs/otlp/#json-protobuf-encoding>.
use crate::{
    content::Content,
    view::{Record, State, Status},
    Error, Result,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;

/// GenAI conventions are Development; every exported resource carries this pin.
pub const GEN_AI_CONVENTIONS: &str = "6fd0d763a092db245a54104fc1124031eb3c51d2";
fn attr(key: &str, value: impl Into<String>) -> Value {
    json!({"key": key, "value": {"stringValue": value.into()}})
}
fn integer_attr(key: &str, value: u64) -> Value {
    json!({"key":key,"value":{"intValue":value.to_string()}})
}
fn identity(value: &str, bytes: usize) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))[..bytes * 2].into()
}
fn nanos(ms: u64) -> Result<String> {
    ms.checked_mul(1_000_000)
        .map(|n| n.to_string())
        .ok_or(Error::Overflow)
}
fn terminal(status: &Status) -> bool {
    matches!(
        status,
        Status::Succeeded | Status::Failed | Status::Cancelled | Status::Interrupted
    )
}
fn content_attrs(attributes: &mut Vec<Value>, prefix: &str, content: &Content) {
    attributes.push(attr(&format!("battersea.{prefix}.sha256"), &content.sha256));
    attributes.push(integer_attr(
        &format!("battersea.{prefix}.byte_length"),
        content.byte_length,
    ));
    if let Some(masked) = &content.masked {
        attributes.push(attr(&format!("battersea.{prefix}.masked"), masked));
    }
}
fn histogram(
    name: &str,
    unit: &str,
    values: &[f64],
    record: &Record,
    attributes: &[Value],
) -> Result<Value> {
    Ok(
        json!({"name":name,"unit":unit,"histogram":{"aggregationTemporality":1,"dataPoints":[{
            "attributes":attributes,"startTimeUnixNano":nanos(record.started_at_ms)?,"timeUnixNano":nanos(record.updated_at_ms)?,
            "count":values.len().to_string(),"sum":values.iter().sum::<f64>(),"bucketCounts":[values.len().to_string()],"explicitBounds":[]
        }]}}),
    )
}
#[derive(Debug, Clone)]
pub struct Batch {
    pub traces: Value,
    pub metrics: Value,
}
impl Batch {
    /// Call once per terminal record. A retained snapshot is for the gauge only;
    /// replaying terminal records would duplicate delta histogram measurements.
    pub fn from_records(
        service: &str,
        completed: &[Record],
        in_flight: u64,
        now_ms: u64,
    ) -> Result<Self> {
        let resource = json!({"attributes":[attr("service.name",service),attr("battersea.gen_ai.conventions",GEN_AI_CONVENTIONS)]});
        let scope = json!({"name":"battersea-telemetry","version":env!("CARGO_PKG_VERSION")});
        let mut spans = vec![];
        let mut metrics = vec![];
        for record in completed {
            let (status, operation, name, kind) = match &record.state {
                State::Activation { status } => (
                    status,
                    "invoke_workflow",
                    format!("invoke_workflow {}", record.context.flow_key),
                    1,
                ),
                State::Node { status, .. } => (
                    status,
                    "node",
                    format!(
                        "node {}",
                        record.context.node_id.as_deref().unwrap_or(&record.id)
                    ),
                    1,
                ),
                State::Tool { status, name, .. } => {
                    (status, "execute_tool", format!("execute_tool {name}"), 1)
                }
                State::Request {
                    status,
                    operation,
                    model,
                    ..
                } => (
                    status,
                    operation.as_str(),
                    format!("{operation} {model}"),
                    3,
                ),
                _ => continue,
            };
            if !terminal(status) {
                continue;
            }
            let mut attributes = vec![
                attr("battersea.activation.id", &record.context.activation_id),
                attr("battersea.flow.key", &record.context.flow_key),
            ];
            if operation != "node" {
                attributes.push(attr("gen_ai.operation.name", operation));
            }
            if let Some(session) = &record.context.session_id {
                attributes.push(attr("gen_ai.conversation.id", session));
            }
            let mut metric_attributes = if operation == "node" {
                vec![]
            } else {
                vec![attr("gen_ai.operation.name", operation)]
            };
            if *status != Status::Succeeded {
                let error = match &record.state {
                    State::Node {
                        error_code: Some(code),
                        ..
                    } => code.clone(),
                    _ => format!("{status:?}").to_lowercase(),
                };
                attributes.push(attr("error.type", &error));
                metric_attributes.push(attr("error.type", error));
            }
            if let State::Tool { name, call_id, .. } = &record.state {
                attributes.push(attr("gen_ai.tool.name", name));
                attributes.push(attr("gen_ai.tool.call.id", call_id));
            }
            if let State::Request {
                provider,
                model,
                usage,
                input,
                output,
                cost,
                first_chunk_ms,
                chunk_timing,
                elapsed_ms,
                ..
            } = &record.state
            {
                for a in [
                    attr("gen_ai.provider.name", provider),
                    attr("gen_ai.request.model", model),
                ] {
                    attributes.push(a.clone());
                    metric_attributes.push(a);
                }
                for (key, name, count) in [
                    (
                        "gen_ai.usage.input_tokens",
                        "gen_ai.client.inference.usage.input_tokens",
                        usage.input_tokens,
                    ),
                    (
                        "gen_ai.usage.output_tokens",
                        "gen_ai.client.inference.usage.output_tokens",
                        usage.output_tokens,
                    ),
                    (
                        "gen_ai.usage.cache_read.input_tokens",
                        "gen_ai.client.inference.usage.cache_read.input_tokens",
                        usage.cached_input_tokens,
                    ),
                    (
                        "gen_ai.usage.cache_write.input_tokens",
                        "gen_ai.client.inference.usage.cache_write.input_tokens",
                        usage.cache_write_input_tokens,
                    ),
                ] {
                    if let Some(count) = count {
                        attributes.push(integer_attr(key, count));
                        metrics.push(histogram(
                            name,
                            "{token}",
                            &[count as f64],
                            record,
                            &metric_attributes,
                        )?);
                    }
                }
                if let Some(input) = input {
                    content_attrs(&mut attributes, "input", input);
                }
                if let Some(output) = output {
                    content_attrs(&mut attributes, "output", output);
                }
                let source = match cost.as_ref() {
                    crate::accounting::Cost::Unknown { .. } => "unknown",
                    crate::accounting::Cost::ProviderReported { .. } => "provider_reported",
                    crate::accounting::Cost::Calculated { .. } => "calculated",
                };
                attributes.push(attr("battersea.cost.source", source));
                if let Some(amount) = cost.amount() {
                    attributes.push(integer_attr("battersea.cost.micros", amount.micros));
                    attributes.push(attr("battersea.cost.currency", &amount.currency));
                }
                metrics.push(histogram(
                    "gen_ai.client.inference.duration",
                    "s",
                    &[*elapsed_ms as f64 / 1000.0],
                    record,
                    &metric_attributes,
                )?);
                if let Some(first) = first_chunk_ms {
                    metrics.push(histogram(
                        "gen_ai.client.inference.time_to_first_chunk",
                        "s",
                        &[*first as f64 / 1000.0],
                        record,
                        &metric_attributes,
                    )?);
                }
                if chunk_timing.count > 0 {
                    let mut point = json!({"attributes":metric_attributes,"startTimeUnixNano":nanos(record.started_at_ms)?,"timeUnixNano":nanos(record.updated_at_ms)?,"count":chunk_timing.count.to_string(),"sum":chunk_timing.total_ms as f64 / 1000.0,"bucketCounts":[chunk_timing.count.to_string()],"explicitBounds":[]});
                    if let Some(minimum) = chunk_timing.minimum_ms {
                        point["min"] = json!(minimum as f64 / 1000.0);
                    }
                    if let Some(maximum) = chunk_timing.maximum_ms {
                        point["max"] = json!(maximum as f64 / 1000.0);
                    }
                    metrics.push(json!({"name":"gen_ai.client.inference.time_per_output_chunk","unit":"s","histogram":{"aggregationTemporality":1,"dataPoints":[point]}}));
                }
            } else if operation != "node" {
                metrics.push(histogram(
                    &format!("gen_ai.{operation}.duration"),
                    "s",
                    &[record.updated_at_ms.saturating_sub(record.started_at_ms) as f64 / 1000.0],
                    record,
                    &metric_attributes,
                )?);
            }
            let parent = match &record.state {
                State::Activation { .. } => String::new(),
                State::Node { .. } => {
                    identity(&activation_resource_id(&record.context.activation_id), 8)
                }
                _ => identity(
                    &record
                        .context
                        .node_id
                        .as_ref()
                        .map(|node| node_record_id(&record.context.activation_id, node))
                        .unwrap_or_else(|| activation_resource_id(&record.context.activation_id)),
                    8,
                ),
            };
            spans.push(json!({"traceId":identity(&record.context.activation_id,16),"spanId":identity(&record.id,8),"parentSpanId":parent,"name":name,"kind":kind,
                "startTimeUnixNano":nanos(record.started_at_ms)?,"endTimeUnixNano":nanos(record.updated_at_ms)?,"attributes":attributes,"status":{"code":if *status == Status::Succeeded { 1 } else { 2 }}}));
        }
        metrics.push(json!({"name":"battersea.provider.requests.in_flight","unit":"{request}","gauge":{"dataPoints":[{"timeUnixNano":nanos(now_ms)?,"asInt":in_flight.to_string()}]}}));
        Ok(Self {
            traces: json!({"resourceSpans":[{"resource":resource,"scopeSpans":[{"scope":scope,"spans":spans}]}]}),
            metrics: json!({"resourceMetrics":[{"resource":resource,"scopeMetrics":[{"scope":scope,"metrics":metrics}]}]}),
        })
    }
}
pub fn activation_resource_id(activation: &str) -> String {
    format!("activation:{activation}")
}
pub fn node_record_id(activation: &str, node: &str) -> String {
    format!("node:{activation}:{node}")
}

#[derive(Debug, Clone)]
pub struct Exporter {
    client: reqwest::Client,
    endpoint: String,
    max_batch_bytes: usize,
}
impl Exporter {
    pub fn new(endpoint: &str, timeout: Duration, max_batch_bytes: usize) -> Result<Self> {
        let url = reqwest::Url::parse(endpoint).map_err(|e| Error::Invalid(e.to_string()))?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || timeout.is_zero()
            || max_batch_bytes == 0
        {
            return Err(Error::Invalid("Invalid OTLP endpoint or capacity".into()));
        }
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| Error::Invalid(e.to_string()))?;
        Ok(Self {
            client,
            endpoint: endpoint.trim_end_matches('/').into(),
            max_batch_bytes,
        })
    }
    /// No detached work and no implicit retry: the owner observes both partial
    /// success and transport failure and decides whether a retry is appropriate.
    pub async fn export(&self, batch: &Batch) -> Result<()> {
        let traces =
            serde_json::to_vec(&batch.traces).map_err(|e| Error::Invalid(e.to_string()))?;
        let metrics =
            serde_json::to_vec(&batch.metrics).map_err(|e| Error::Invalid(e.to_string()))?;
        if traces.len().saturating_add(metrics.len()) > self.max_batch_bytes {
            return Err(Error::Capacity);
        }
        self.send("traces", traces).await?;
        self.send("metrics", metrics).await
    }
    async fn send(&self, signal: &str, body: Vec<u8>) -> Result<()> {
        let mut response = self
            .client
            .post(format!("{}/v1/{signal}", self.endpoint))
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| Error::Delivery(e.to_string()))?;
        if !response.status().is_success() {
            return Err(Error::Delivery(format!(
                "OTLP {signal}: HTTP {}",
                response.status()
            )));
        }
        let mut bytes = vec![];
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| Error::Delivery(e.to_string()))?
        {
            if bytes.len().saturating_add(chunk.len()) > 32_768 {
                return Err(Error::Capacity);
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|e| Error::Delivery(format!("Invalid OTLP response: {e}")))?;
        if let Some(partial) = value.get("partialSuccess") {
            for key in ["rejectedSpans", "rejectedDataPoints"] {
                if partial.get(key).is_some_and(|v| {
                    v.as_u64().unwrap_or_else(|| {
                        v.as_str().and_then(|v| v.parse().ok()).unwrap_or(u64::MAX)
                    }) > 0
                }) {
                    return Err(Error::Delivery(format!("OTLP {signal} rejected records")));
                }
            }
            if partial
                .get("errorMessage")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
            {
                return Err(Error::Delivery(format!(
                    "OTLP {signal} reported partial success"
                )));
            }
        }
        Ok(())
    }
}
