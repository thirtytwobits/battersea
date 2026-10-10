//! Import operator-supplied LiteLLM price snapshots without a network dependency.
use crate::{
    accounting::{Price, PriceCatalogue},
    Error, Result,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

// USD/token -> currency micros/million tokens, rounded upwards without floats.
fn rate(value: &Value) -> Result<u64> {
    let text = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    let (mantissa, exponent) = text
        .split_once(['e', 'E'])
        .map_or((text.as_str(), 0), |(m, e)| {
            (m, e.parse::<i32>().unwrap_or(i32::MIN))
        });
    if exponent == i32::MIN || mantissa.starts_with('-') {
        return Err(Error::Invalid("Invalid price rate".into()));
    }
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if whole.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|v| v.is_ascii_digit())
    {
        return Err(Error::Invalid("Invalid price rate".into()));
    }
    let digits: u128 = format!("{whole}{fraction}")
        .parse()
        .map_err(|_| Error::Overflow)?;
    let scale = 12i32
        .checked_add(exponent)
        .and_then(|v| v.checked_sub(fraction.len() as i32))
        .ok_or(Error::Overflow)?;
    let scaled = if scale >= 0 {
        digits
            .checked_mul(10u128.checked_pow(scale as u32).ok_or(Error::Overflow)?)
            .ok_or(Error::Overflow)?
    } else {
        let divisor = 10u128
            .checked_pow(scale.unsigned_abs())
            .ok_or(Error::Overflow)?;
        digits.checked_add(divisor - 1).ok_or(Error::Overflow)? / divisor
    };
    u64::try_from(scaled).map_err(|_| Error::Overflow)
}
impl PriceCatalogue {
    pub fn from_litellm_json(input: &[u8], effective_date: String) -> Result<Self> {
        let data: BTreeMap<String, Value> =
            serde_json::from_slice(input).map_err(|e| Error::Invalid(e.to_string()))?;
        let mut prices = BTreeMap::new();
        for (model, config) in data {
            let Some(fields) = config.as_object() else {
                continue;
            };
            if fields.get("mode").and_then(Value::as_str) != Some("chat") {
                continue;
            }
            // Tiered or modality-specific billing cannot be represented by this
            // tariff. Omit it so budget admission requires an explicit price.
            if fields.keys().any(|key| {
                key.contains("cost")
                    && (key.contains("above")
                        || key.contains("audio")
                        || key.contains("image")
                        || key.contains("video")
                        || key.contains("character")
                        || key.contains("second"))
            }) {
                continue;
            }
            let (Some(provider), Some(input), Some(output)) = (
                fields.get("litellm_provider").and_then(Value::as_str),
                fields.get("input_cost_per_token"),
                fields.get("output_cost_per_token"),
            ) else {
                continue;
            };
            let price = Price {
                currency: "USD".into(),
                input_micros_per_million: rate(input)?,
                output_micros_per_million: rate(output)?,
                cached_input_micros_per_million: fields
                    .get("cache_read_input_token_cost")
                    .map(rate)
                    .transpose()?,
                cache_write_input_micros_per_million: fields
                    .get("cache_creation_input_token_cost")
                    .map(rate)
                    .transpose()?,
                request_micros: 0,
            };
            let model = model
                .strip_prefix(&format!("{provider}/"))
                .unwrap_or(&model);
            if prices
                .insert(format!("{provider}/{model}"), price)
                .is_some()
            {
                return Err(Error::Invalid("Duplicate provider/model price".into()));
            }
        }
        let result = Self {
            version: format!("sha256:{:x}", Sha256::digest(input)),
            source: "LiteLLM model_prices_and_context_window.json".into(),
            effective_date,
            prices,
        };
        result.validate()?;
        Ok(result)
    }
}
