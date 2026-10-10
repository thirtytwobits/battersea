use crate::{accounting::Money, view::Retention, Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TelemetryConfig {
    pub retention: Retention,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "PriceSource")]
    pub price_catalogue: Option<PriceSource>,
    pub budgets: BTreeMap<String, BudgetRule>,
    pub max_accounting_requests: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "OtlpConfig")]
    pub otlp: Option<OtlpConfig>,
    /// Exact strings replaced before opt-in content enters any observation.
    /// Omission disables content capture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "MaskConfig")]
    pub capture: Option<MaskConfig>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "format", rename_all = "snake_case", deny_unknown_fields)]
pub enum PriceSource {
    Battersea {
        path: String,
    },
    Litellm {
        path: String,
        effective_date: String,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BudgetRule {
    pub limit: Money,
    /// Empty selectors apply to every request.
    pub backends: Vec<String>,
    pub flows: Vec<String>,
}
impl BudgetRule {
    pub fn applies(&self, backend: &str, flow: &str) -> bool {
        (self.backends.is_empty() || self.backends.iter().any(|s| s == backend))
            && (self.flows.is_empty() || self.flows.iter().any(|s| s == flow))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OtlpConfig {
    pub endpoint: String,
    pub timeout_ms: u64,
    pub max_batch_bytes: usize,
    pub max_queued_batches: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MaskConfig {
    pub replacements: BTreeMap<String, String>,
    pub max_content_bytes: usize,
}
impl MaskConfig {
    pub fn mask(&self, bytes: &[u8]) -> String {
        let mut text = String::from_utf8_lossy(bytes).into_owned();
        for (secret, replacement) in &self.replacements {
            text = text.replace(secret, replacement);
        }
        let mut length = text.len().min(self.max_content_bytes);
        while !text.is_char_boundary(length) {
            length -= 1;
        }
        text.truncate(length);
        text
    }
}
impl TelemetryConfig {
    pub fn validate(&self) -> Result<()> {
        crate::schema::validate_integers(
            &serde_json::to_value(self).map_err(|e| Error::Invalid(e.to_string()))?,
        )?;
        crate::view::View::new("validation".into(), self.retention)?;
        if self.max_accounting_requests == 0 {
            return Err(Error::Invalid(
                "Accounting capacity must be positive".into(),
            ));
        }
        for rule in self.budgets.values() {
            rule.limit.validate()?;
        }
        if !self.budgets.is_empty() && self.price_catalogue.is_none() {
            return Err(Error::Invalid("Budgets require a price catalogue".into()));
        }
        if let Some(mask) = &self.capture {
            if mask.max_content_bytes == 0 || mask.replacements.keys().any(|key| key.is_empty()) {
                return Err(Error::Invalid(
                    "Invalid content masking configuration".into(),
                ));
            }
        }
        if let Some(otlp) = &self.otlp {
            if otlp.max_queued_batches == 0 {
                return Err(Error::Invalid(
                    "Exporter queue must be bounded and positive".into(),
                ));
            }
            crate::otlp::Exporter::new(
                &otlp.endpoint,
                std::time::Duration::from_millis(otlp.timeout_ms),
                otlp.max_batch_bytes,
            )?;
        }
        Ok(())
    }
}
