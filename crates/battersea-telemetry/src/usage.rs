use crate::{accounting::Usage, Error, Result};
use battersea_model::adapter::EngineTokenUsage;
use std::collections::BTreeMap;

/// Provider usage is a cumulative snapshot within one turn. Tool turns have
/// distinct indices, so repeated snapshots cannot double-charge a request.
#[derive(Debug, Clone)]
pub struct UsageAccumulator {
    turns: BTreeMap<u32, EngineTokenUsage>,
    max_turns: u32,
}
impl UsageAccumulator {
    pub fn new(max_turns: u32) -> Result<Self> {
        if max_turns == 0 {
            return Err(Error::Invalid(
                "Usage needs a finite positive turn bound".into(),
            ));
        }
        Ok(Self {
            turns: BTreeMap::new(),
            max_turns,
        })
    }
    pub fn observe(&mut self, usage: EngineTokenUsage) -> Result<()> {
        if usage.turn_index >= self.max_turns || usage.turn_index as usize > self.turns.len() {
            return Err(Error::Capacity);
        }
        if self
            .turns
            .keys()
            .next_back()
            .is_some_and(|last| usage.turn_index < *last)
        {
            return Err(Error::Conflict);
        }
        let previous = self
            .turns
            .get(&usage.turn_index)
            .copied()
            .unwrap_or_default();
        let merge = |old: Option<u64>, next: Option<u64>| -> Result<Option<u64>> {
            if old.zip(next).is_some_and(|(old, next)| next < old) {
                return Err(Error::Conflict);
            }
            Ok(next.or(old))
        };
        let next = EngineTokenUsage {
            turn_index: usage.turn_index,
            input_tokens: merge(previous.input_tokens, usage.input_tokens)?,
            output_tokens: merge(previous.output_tokens, usage.output_tokens)?,
            total_tokens: merge(previous.total_tokens, usage.total_tokens)?,
            cached_input_tokens: merge(previous.cached_input_tokens, usage.cached_input_tokens)?,
            cache_write_input_tokens: merge(
                previous.cache_write_input_tokens,
                usage.cache_write_input_tokens,
            )?,
            reasoning_output_tokens: merge(
                previous.reasoning_output_tokens,
                usage.reasoning_output_tokens,
            )?,
        };
        self.turns.insert(usage.turn_index, next);
        Ok(())
    }
    /// Price each provider turn independently, including its request fee and rounding.
    pub fn cost(
        &self,
        catalogue: &crate::accounting::PriceCatalogue,
        provider: &str,
        model: &str,
    ) -> Result<crate::accounting::Cost> {
        use crate::accounting::{Cost, Money};
        if self.turns.is_empty() {
            return Ok(Cost::default());
        }
        let mut total: Option<Money> = None;
        for turn in self.turns.values() {
            let usage = Usage {
                input_tokens: turn.input_tokens,
                output_tokens: turn.output_tokens,
                cached_input_tokens: turn.cached_input_tokens,
                cache_write_input_tokens: turn.cache_write_input_tokens,
            };
            let cost = catalogue.cost(provider, model, &usage, None)?;
            let Some(amount) = cost.amount() else {
                return Ok(cost);
            };
            match &mut total {
                Some(total) => {
                    if total.currency != amount.currency {
                        return Err(Error::Conflict);
                    }
                    total.micros = total
                        .micros
                        .checked_add(amount.micros)
                        .ok_or(Error::Overflow)?;
                    total.validate()?;
                }
                None => total = Some(amount.clone()),
            }
        }
        Ok(Cost::Calculated {
            amount: total.ok_or(Error::Conflict)?,
            catalogue_version: catalogue.version.clone(),
            effective_date: catalogue.effective_date.clone(),
        })
    }
    pub fn totals(&self) -> Result<Usage> {
        if self.turns.is_empty() {
            return Ok(Usage::default());
        }
        let sum = |field: fn(&EngineTokenUsage) -> Option<u64>| -> Result<Option<u64>> {
            self.turns
                .values()
                .try_fold(Some(0u64), |total, usage| match (total, field(usage)) {
                    (Some(a), Some(b)) => a.checked_add(b).map(Some).ok_or(Error::Overflow),
                    _ => Ok(None),
                })
        };
        Ok(Usage {
            input_tokens: sum(|u| u.input_tokens)?,
            output_tokens: sum(|u| u.output_tokens)?,
            cached_input_tokens: sum(|u| u.cached_input_tokens)?,
            cache_write_input_tokens: sum(|u| u.cache_write_input_tokens)?,
        })
    }
}
