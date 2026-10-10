use crate::{Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Money {
    pub currency: String,
    pub micros: u64,
}
impl Money {
    pub fn validate(&self) -> Result<()> {
        if self.currency.len() != 3
            || !self.currency.bytes().all(|b| b.is_ascii_uppercase())
            || self.micros > 9_007_199_254_740_991
        {
            return Err(Error::Invalid(
                "Money requires a currency code and an exact JSON integer".into(),
            ));
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    /// Inclusive of cache reads and writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u64")]
    pub input_tokens: Option<u64>,
    /// Inclusive of billed reasoning tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u64")]
    pub output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u64")]
    pub cached_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u64")]
    pub cache_write_input_tokens: Option<u64>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum Cost {
    Unknown {
        reason: String,
    },
    ProviderReported {
        amount: Money,
    },
    Calculated {
        amount: Money,
        catalogue_version: String,
        effective_date: String,
    },
}
impl Cost {
    pub fn amount(&self) -> Option<&Money> {
        match self {
            Self::Unknown { .. } => None,
            Self::ProviderReported { amount } | Self::Calculated { amount, .. } => Some(amount),
        }
    }
}
impl Default for Cost {
    fn default() -> Self {
        Self::Unknown {
            reason: "Provider has not reported complete billable usage".into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Price {
    pub currency: String,
    pub input_micros_per_million: u64,
    pub output_micros_per_million: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u64")]
    pub cached_input_micros_per_million: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u64")]
    pub cache_write_input_micros_per_million: Option<u64>,
    pub request_micros: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PriceCatalogue {
    pub version: String,
    pub source: String,
    pub effective_date: String,
    /// Keys are provider/model, not a display name or configured backend alias.
    pub prices: BTreeMap<String, Price>,
}
impl PriceCatalogue {
    pub fn cost(
        &self,
        provider: &str,
        model: &str,
        usage: &Usage,
        reported: Option<Money>,
    ) -> Result<Cost> {
        if let Some(amount) = reported {
            amount.validate()?;
            return Ok(Cost::ProviderReported { amount });
        }
        let Some(price) = self.prices.get(&format!("{provider}/{model}")) else {
            return Ok(Cost::Unknown {
                reason: "No matching price".into(),
            });
        };
        let Some(amount) = price.calculate(usage)? else {
            return Ok(Cost::default());
        };
        Ok(Cost::Calculated {
            amount,
            catalogue_version: self.version.clone(),
            effective_date: self.effective_date.clone(),
        })
    }
    pub fn validate(&self) -> Result<()> {
        crate::schema::validate_integers(
            &serde_json::to_value(self).map_err(|e| Error::Invalid(e.to_string()))?,
        )?;
        if self.version.is_empty()
            || self.source.is_empty()
            || self.effective_date.len() != 10
            || chrono_date_invalid(&self.effective_date)
        {
            return Err(Error::Invalid(
                "Catalogue requires source, version and ISO effective date".into(),
            ));
        }
        for (key, price) in &self.prices {
            if !key.contains('/') {
                return Err(Error::Invalid(
                    "Price keys must identify provider/model".into(),
                ));
            }
            Money {
                currency: price.currency.clone(),
                micros: price.request_micros,
            }
            .validate()?;
        }
        Ok(())
    }
}
fn chrono_date_invalid(date: &str) -> bool {
    let p: Vec<_> = date.split('-').collect();
    if p.len() != 3 {
        return true;
    }
    let (Ok(y), Ok(m), Ok(d)) = (
        p[0].parse::<u32>(),
        p[1].parse::<u32>(),
        p[2].parse::<u32>(),
    ) else {
        return true;
    };
    let max = match m {
        4 | 6 | 9 | 11 => 30,
        2 if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        2 => 28,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    y == 0 || d == 0 || d > max
}
impl Price {
    fn amount(&self, numerator: u128) -> Result<Money> {
        let rounded = numerator.checked_add(999_999).ok_or(Error::Overflow)? / 1_000_000;
        let micros = u64::try_from(rounded)
            .map_err(|_| Error::Overflow)?
            .checked_add(self.request_micros)
            .ok_or(Error::Overflow)?;
        let amount = Money {
            currency: self.currency.clone(),
            micros,
        };
        amount.validate()?;
        Ok(amount)
    }
    pub fn calculate(&self, usage: &Usage) -> Result<Option<Money>> {
        let (Some(input), Some(output)) = (usage.input_tokens, usage.output_tokens) else {
            return Ok(None);
        };
        // Distinct cache prices require explicit counts, including known zero.
        let cached = match (
            self.cached_input_micros_per_million,
            usage.cached_input_tokens,
        ) {
            (Some(_), None) => return Ok(None),
            (_, n) => n.unwrap_or(0),
        };
        let written = match (
            self.cache_write_input_micros_per_million,
            usage.cache_write_input_tokens,
        ) {
            (Some(_), None) => return Ok(None),
            (_, n) => n.unwrap_or(0),
        };
        let regular = input
            .checked_sub(cached)
            .and_then(|n| n.checked_sub(written))
            .ok_or_else(|| Error::Invalid("Cache usage exceeds input usage".into()))?;
        let terms = [
            (regular, self.input_micros_per_million),
            (
                cached,
                self.cached_input_micros_per_million
                    .unwrap_or(self.input_micros_per_million),
            ),
            (
                written,
                self.cache_write_input_micros_per_million
                    .unwrap_or(self.input_micros_per_million),
            ),
            (output, self.output_micros_per_million),
        ];
        let numerator = terms.into_iter().try_fold(0u128, |sum, (count, rate)| {
            sum.checked_add(u128::from(count) * u128::from(rate))
                .ok_or(Error::Overflow)
        })?;
        self.amount(numerator).map(Some)
    }
    /// Host supplies a bound covering every provider turn, including tool rounds.
    pub fn ceiling(&self, input_tokens: u64, output_tokens: u64, requests: u32) -> Result<Money> {
        if requests == 0 {
            return Err(Error::Invalid("Request bound must be positive".into()));
        }
        let rate = self
            .input_micros_per_million
            .max(self.cached_input_micros_per_million.unwrap_or(0))
            .max(self.cache_write_input_micros_per_million.unwrap_or(0));
        let numerator = (u128::from(input_tokens) * u128::from(rate))
            .checked_add(u128::from(output_tokens) * u128::from(self.output_micros_per_million))
            .ok_or(Error::Overflow)?;
        let mut amount = self.amount(numerator)?;
        amount.micros = amount
            .micros
            .checked_mul(u64::from(requests))
            .ok_or(Error::Overflow)?;
        amount.validate()?;
        Ok(amount)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    pub limit: Money,
    pub spent_micros: u64,
    pub reserved_micros: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ReservationState {
    Reserved,
    Ambiguous,
    Settled { cost: Cost },
    Released { reason: String },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Reservation {
    pub request_id: String,
    pub budgets: Vec<String>,
    pub ceiling: Money,
    pub state: ReservationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub reconciliation: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    pub version: u32,
    pub budgets: BTreeMap<String, Budget>,
    pub requests: BTreeMap<String, Reservation>,
    pub max_requests: usize,
}
impl Ledger {
    pub fn new(limits: BTreeMap<String, Money>, max_requests: usize) -> Result<Self> {
        if max_requests == 0 {
            return Err(Error::Invalid(
                "Accounting retention must be positive".into(),
            ));
        }
        let mut budgets = BTreeMap::new();
        for (name, limit) in limits {
            limit.validate()?;
            if name.is_empty() {
                return Err(Error::Invalid("Empty budget identity".into()));
            }
            budgets.insert(
                name,
                Budget {
                    limit,
                    spent_micros: 0,
                    reserved_micros: 0,
                },
            );
        }
        Ok(Self {
            version: 1,
            budgets,
            requests: BTreeMap::new(),
            max_requests,
        })
    }
    /// Run under the host's admission lock. Persistence occurs before publication;
    /// persistence failure does not admit a request or mutate this ledger.
    fn transact<T>(
        &mut self,
        change: impl FnOnce(&mut Self) -> Result<T>,
        persist: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<T> {
        let mut next = self.clone();
        let result = change(&mut next)?;
        crate::schema::validate_integers(
            &serde_json::to_value(&next).map_err(|e| Error::Invalid(e.to_string()))?,
        )?;
        persist(&next)?;
        *self = next;
        Ok(result)
    }
    pub fn reserve(
        &mut self,
        request_id: &str,
        mut budgets: Vec<String>,
        ceiling: Money,
        persist: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<()> {
        ceiling.validate()?;
        budgets.sort();
        if request_id.is_empty() || budgets.is_empty() || budgets.windows(2).any(|v| v[0] == v[1]) {
            return Err(Error::Invalid(
                "Admission requires unique budget and request identities".into(),
            ));
        }
        self.transact(
            |next| {
                // A replay must reconcile, never dispatch the request again.
                if next.requests.contains_key(request_id) {
                    return Err(Error::Conflict);
                }
                if next.requests.len() >= next.max_requests {
                    return Err(Error::Capacity);
                }
                for id in &budgets {
                    let budget = next
                        .budgets
                        .get_mut(id)
                        .ok_or_else(|| Error::Budget(format!("Unknown budget {id}")))?;
                    if budget.limit.currency != ceiling.currency {
                        return Err(Error::Budget("Currency mismatch".into()));
                    }
                    let reserved = budget
                        .reserved_micros
                        .checked_add(ceiling.micros)
                        .ok_or(Error::Overflow)?;
                    if budget
                        .spent_micros
                        .checked_add(reserved)
                        .ok_or(Error::Overflow)?
                        > budget.limit.micros
                    {
                        return Err(Error::Budget(format!("Limit exceeded for {id}")));
                    }
                    budget.reserved_micros = reserved;
                }
                next.requests.insert(
                    request_id.into(),
                    Reservation {
                        request_id: request_id.into(),
                        budgets,
                        ceiling,
                        state: ReservationState::Reserved,
                        reconciliation: None,
                    },
                );
                Ok(())
            },
            persist,
        )
    }
    pub fn settle(
        &mut self,
        request_id: &str,
        cost: Cost,
        persist: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<()> {
        self.transact(
            |next| {
                let reservation = next.requests.get_mut(request_id).ok_or(Error::Conflict)?;
                if let ReservationState::Settled { cost: old } = &reservation.state {
                    return if old == &cost {
                        Ok(())
                    } else {
                        Err(Error::Conflict)
                    };
                }
                if matches!(reservation.state, ReservationState::Released { .. }) {
                    return Err(Error::Conflict);
                }
                let Some(amount) = cost.amount() else {
                    reservation.state = ReservationState::Ambiguous;
                    return Ok(());
                };
                amount.validate()?;
                if amount.currency != reservation.ceiling.currency {
                    return Err(Error::Budget("Settlement currency mismatch".into()));
                }
                for id in &reservation.budgets {
                    let budget = next.budgets.get_mut(id).ok_or(Error::Conflict)?;
                    budget.reserved_micros = budget
                        .reserved_micros
                        .checked_sub(reservation.ceiling.micros)
                        .ok_or(Error::Conflict)?;
                    // An unexpected provider overrun is still charged in full. Future
                    // admission is refused; accounting must not hide actual spend.
                    budget.spent_micros = budget
                        .spent_micros
                        .checked_add(amount.micros)
                        .ok_or(Error::Overflow)?;
                }
                reservation.state = ReservationState::Settled { cost };
                Ok(())
            },
            persist,
        )
    }
    /// Explicit host reconciliation, with evidence that no further charge is due.
    pub fn release(
        &mut self,
        request_id: &str,
        reason: String,
        persist: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<()> {
        if reason.trim().is_empty() {
            return Err(Error::Invalid("Reconciliation requires a reason".into()));
        }
        self.transact(
            |next| {
                let reservation = next.requests.get_mut(request_id).ok_or(Error::Conflict)?;
                if let ReservationState::Released { reason: old } = &reservation.state {
                    return if old == &reason {
                        Ok(())
                    } else {
                        Err(Error::Conflict)
                    };
                }
                if matches!(reservation.state, ReservationState::Settled { .. }) {
                    return Err(Error::Conflict);
                }
                for id in &reservation.budgets {
                    let budget = next.budgets.get_mut(id).ok_or(Error::Conflict)?;
                    budget.reserved_micros = budget
                        .reserved_micros
                        .checked_sub(reservation.ceiling.micros)
                        .ok_or(Error::Conflict)?;
                }
                reservation.state = ReservationState::Released { reason };
                Ok(())
            },
            persist,
        )
    }
    pub fn reconcile(
        &mut self,
        request_id: &str,
        charge: Option<Money>,
        reason: String,
        persist: impl FnOnce(&Self) -> Result<()>,
    ) -> Result<()> {
        if reason.trim().is_empty() {
            return Err(Error::Invalid("Reconciliation requires evidence".into()));
        }
        self.transact(
            |next| {
                let reservation = next.requests.get(request_id).ok_or(Error::Conflict)?;
                if reservation
                    .reconciliation
                    .as_ref()
                    .is_some_and(|old| old != &reason)
                {
                    return Err(Error::Conflict);
                }
                match charge {
                    Some(amount) => {
                        next.settle(request_id, Cost::ProviderReported { amount }, |_| Ok(()))?
                    }
                    None => next.release(request_id, reason.clone(), |_| Ok(()))?,
                }
                next.requests
                    .get_mut(request_id)
                    .ok_or(Error::Conflict)?
                    .reconciliation = Some(reason);
                Ok(())
            },
            persist,
        )
    }
    pub fn restore(bytes: &[u8]) -> Result<Self> {
        let ledger: Self =
            serde_json::from_slice(bytes).map_err(|e| Error::Invalid(e.to_string()))?;
        crate::schema::validate_integers(
            &serde_json::to_value(&ledger).map_err(|e| Error::Invalid(e.to_string()))?,
        )?;
        if ledger.version != 1
            || ledger.max_requests == 0
            || ledger.requests.len() > ledger.max_requests
        {
            return Err(Error::Invalid(
                "Unsupported or invalid accounting journal".into(),
            ));
        }
        // Reconstruct totals from retained request identities; reject corrupt journals.
        let mut expected = Self::new(
            ledger
                .budgets
                .iter()
                .map(|(id, b)| (id.clone(), b.limit.clone()))
                .collect(),
            ledger.max_requests,
        )?;
        for (id, reservation) in &ledger.requests {
            if id != &reservation.request_id {
                return Err(Error::Conflict);
            }
            let mut budget_ids = reservation.budgets.clone();
            budget_ids.sort();
            budget_ids.dedup();
            if budget_ids.len() != reservation.budgets.len() || budget_ids.is_empty() {
                return Err(Error::Conflict);
            }
            reservation.ceiling.validate()?;
            for budget_id in &budget_ids {
                let budget = expected.budgets.get_mut(budget_id).ok_or(Error::Conflict)?;
                if reservation.ceiling.currency != budget.limit.currency {
                    return Err(Error::Conflict);
                }
                match &reservation.state {
                    ReservationState::Reserved | ReservationState::Ambiguous => {
                        budget.reserved_micros = budget
                            .reserved_micros
                            .checked_add(reservation.ceiling.micros)
                            .ok_or(Error::Overflow)?
                    }
                    ReservationState::Settled { cost } => {
                        let amount = cost.amount().ok_or(Error::Conflict)?;
                        amount.validate()?;
                        if amount.currency != budget.limit.currency {
                            return Err(Error::Conflict);
                        }
                        budget.spent_micros = budget
                            .spent_micros
                            .checked_add(amount.micros)
                            .ok_or(Error::Overflow)?;
                    }
                    ReservationState::Released { reason } if reason.trim().is_empty() => {
                        return Err(Error::Conflict)
                    }
                    ReservationState::Released { .. } => (),
                }
            }
        }
        if expected.budgets != ledger.budgets {
            return Err(Error::Conflict);
        }
        Ok(ledger)
    }
}
