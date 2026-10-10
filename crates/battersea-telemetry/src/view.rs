use crate::{content::Content, Error, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Cursor {
    pub epoch: String,
    /// Decimal string so JavaScript clients preserve every revision.
    pub revision: String,
}
impl Cursor {
    fn at(epoch: &str, revision: u64) -> Self {
        Self {
            epoch: epoch.into(),
            revision: revision.to_string(),
        }
    }
    fn number(&self) -> Result<u64> {
        let number: u64 = self.revision.parse().map_err(|_| Error::ResyncRequired)?;
        if self.revision != number.to_string() {
            return Err(Error::ResyncRequired);
        }
        Ok(number)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Accepted,
    Running,
    Waiting,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
    CompletionPending,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Context {
    pub activation_id: String,
    pub flow_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum State {
    Activation {
        status: Status,
    },
    Node {
        status: Status,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(with = "String")]
        error_code: Option<String>,
    },
    Port {
        port: String,
        token_type: String,
        direction: Direction,
        action: PortAction,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(with = "Content")]
        content: Option<Content>,
    },
    Request {
        request_id: String,
        provider: String,
        model: String,
        operation: String,
        status: Status,
        retry: RetryState,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(with = "Content")]
        input: Option<Box<Content>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(with = "Content")]
        output: Option<Box<Content>>,
        usage: Box<crate::accounting::Usage>,
        cost: Box<crate::accounting::Cost>,
        elapsed_ms: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(with = "u64")]
        first_chunk_ms: Option<u64>,
        chunk_timing: TimingSummary,
    },
    MediaJob {
        job_id: String,
        provider: String,
        status: Status,
        retry: RetryState,
    },
    Watchdog {
        request_id: String,
        elapsed_ms: u64,
        idle_limit_ms: u64,
        timed_out: bool,
    },
    Tool {
        call_id: String,
        name: String,
        status: Status,
    },
    Diagnostic {
        category: String,
        attributes: BTreeMap<String, String>,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(with = "Content")]
        content: Option<Content>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Input,
    Output,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PortAction {
    Emit,
    Receive,
    Skip,
    Drop,
    Close,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RetryState {
    None,
    Scheduled { attempt: u32, at_ms: u64 },
    Ambiguous,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(rename = "RuntimeRecord")]
pub struct Record {
    /// Stable identity of this observed resource, scoped by its activation.
    pub id: String,
    pub context: Context,
    pub started_at_ms: u64,
    pub updated_at_ms: u64,
    pub state: State,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub cursor: Cursor,
    pub records: Vec<Record>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    Upsert { record: Box<Record> },
    Remove { id: String },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Delta {
    pub base: Cursor,
    pub cursor: Cursor,
    pub changes: Vec<Change>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Update {
    Deltas { deltas: Vec<Delta>, cursor: Cursor },
    Resync { snapshot: Snapshot },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Retention {
    pub max_records: usize,
    pub max_record_bytes: usize,
    pub max_delta_bytes: usize,
    pub max_deltas: usize,
    pub max_age_ms: u64,
}
impl Retention {
    fn validate(self) -> Result<Self> {
        if self.max_records == 0
            || self.max_record_bytes == 0
            || self.max_delta_bytes == 0
            || self.max_deltas == 0
            || self.max_age_ms == 0
        {
            return Err(Error::Invalid("Retention limits must be positive".into()));
        }
        Ok(self)
    }
}

/// No timers, tracing subscriber or hidden read-side maintenance.
#[derive(Debug, Clone)]
pub struct View {
    epoch: String,
    revision: u64,
    records: BTreeMap<String, Record>,
    record_sizes: BTreeMap<String, usize>,
    record_bytes: usize,
    delta_bytes: usize,
    deltas: VecDeque<(Delta, usize)>,
    retention: Retention,
    last_write_ms: u64,
}
fn bytes<T: Serialize>(value: &T) -> Result<usize> {
    crate::schema::validate_integers(
        &serde_json::to_value(value).map_err(|e| Error::Invalid(e.to_string()))?,
    )?;
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .map_err(|e| Error::Invalid(e.to_string()))
}
impl View {
    pub fn new(epoch: String, retention: Retention) -> Result<Self> {
        if epoch.is_empty() || epoch.len() > 128 {
            return Err(Error::Invalid("Invalid runtime epoch".into()));
        }
        Ok(Self {
            epoch,
            revision: 0,
            records: BTreeMap::new(),
            record_sizes: BTreeMap::new(),
            record_bytes: 0,
            delta_bytes: 0,
            deltas: VecDeque::new(),
            retention: retention.validate()?,
            last_write_ms: 0,
        })
    }
    pub fn history(&self) -> impl Iterator<Item = &Delta> {
        self.deltas.iter().map(|(delta, _)| delta)
    }
    pub fn get(&self, id: &str) -> Option<&Record> {
        self.records.get(id)
    }
    pub fn cursor(&self) -> Cursor {
        Cursor::at(&self.epoch, self.revision)
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            cursor: Cursor::at(&self.epoch, self.revision),
            records: self.records.values().cloned().collect(),
        }
    }
    pub fn updates(&self, after: &Cursor) -> Update {
        let resync = || Update::Resync {
            snapshot: self.snapshot(),
        };
        let Ok(revision) = after.number() else {
            return resync();
        };
        if after.epoch != self.epoch || revision > self.revision {
            return resync();
        }
        if revision == self.revision {
            return Update::Deltas {
                deltas: vec![],
                cursor: after.clone(),
            };
        }
        let Some(start) = self
            .deltas
            .iter()
            .position(|(delta, _)| &delta.base == after)
        else {
            return resync();
        };
        Update::Deltas {
            deltas: self
                .deltas
                .iter()
                .skip(start)
                .map(|(d, _)| d.clone())
                .collect(),
            cursor: Cursor::at(&self.epoch, self.revision),
        }
    }
    /// Capture must durably accept the complete delta or return failure. A failed
    /// capture leaves the view and cursor unchanged and is returned to the caller.
    pub fn record(
        &mut self,
        record: Record,
        capture: impl FnOnce(&Delta) -> Result<()>,
    ) -> Result<Delta> {
        if record.id.is_empty() || record.started_at_ms > record.updated_at_ms {
            return Err(Error::Invalid("Invalid record identity or time".into()));
        }
        if let Some(old) = self.records.get(&record.id) {
            if old.context != record.context
                || old.started_at_ms != record.started_at_ms
                || old.updated_at_ms > record.updated_at_ms
            {
                return Err(Error::Conflict);
            }
        }
        let now = record.updated_at_ms.max(self.last_write_ms);
        self.commit(Some(record), now, capture)
    }
    pub fn maintain(
        &mut self,
        now_ms: u64,
        capture: impl FnOnce(&Delta) -> Result<()>,
    ) -> Result<Delta> {
        self.commit(None, now_ms.max(self.last_write_ms), capture)
    }
    pub fn in_flight_requests(&self) -> usize {
        self.records
            .values()
            .filter(|r| matches!(&r.state,State::Request { status, .. } if active(status)))
            .count()
    }
    pub fn records(&self) -> impl Iterator<Item = &Record> {
        self.records.values()
    }
    fn commit(
        &mut self,
        record: Option<Record>,
        now: u64,
        capture: impl FnOnce(&Delta) -> Result<()>,
    ) -> Result<Delta> {
        let new_size = record.as_ref().map(bytes).transpose()?.unwrap_or(0);
        if new_size > self.retention.max_record_bytes {
            return Err(Error::Capacity);
        }
        let replacing = record
            .as_ref()
            .and_then(|r| self.record_sizes.get(&r.id))
            .copied();
        let mut total = self.record_bytes - replacing.unwrap_or(0) + new_size;
        let mut count = self.records.len() + usize::from(record.is_some() && replacing.is_none());
        let mut candidates: Vec<_> = self
            .records
            .values()
            .filter(|r| record.as_ref().is_none_or(|new| new.id != r.id))
            .map(|r| {
                (
                    r.updated_at_ms,
                    r.id.clone(),
                    self.record_sizes[&r.id],
                    pinned(r),
                )
            })
            .chain(
                record
                    .iter()
                    .map(|r| (r.updated_at_ms, r.id.clone(), new_size, pinned(r))),
            )
            .collect();
        candidates.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        let mut removals = Vec::new();
        for (updated, id, size, pinned) in candidates {
            if pinned {
                continue;
            }
            if now.saturating_sub(updated) >= self.retention.max_age_ms
                || count > self.retention.max_records
                || total > self.retention.max_record_bytes
            {
                removals.push(id);
                total -= size;
                count -= 1;
            }
        }
        // An active resource cannot disappear merely because terminal history is full.
        if count > self.retention.max_records || total > self.retention.max_record_bytes {
            return Err(Error::Capacity);
        }
        let mut changes = record
            .iter()
            .cloned()
            .map(|record| Change::Upsert {
                record: Box::new(record),
            })
            .collect::<Vec<_>>();
        changes.extend(removals.iter().cloned().map(|id| Change::Remove { id }));
        let revision = self.revision.checked_add(1).ok_or(Error::Overflow)?;
        let delta = Delta {
            base: Cursor::at(&self.epoch, self.revision),
            cursor: Cursor::at(&self.epoch, revision),
            changes,
        };
        let size = bytes(&delta)?;
        capture(&delta)?;
        if let Some(record) = record {
            self.record_sizes.insert(record.id.clone(), new_size);
            self.records.insert(record.id.clone(), record);
        }
        for id in removals {
            self.records.remove(&id);
            self.record_sizes.remove(&id);
        }
        self.record_bytes = total;
        self.revision = revision;
        self.last_write_ms = now;
        self.delta_bytes += size;
        self.deltas.push_back((delta.clone(), size));
        while self.deltas.len() > self.retention.max_deltas
            || self.delta_bytes > self.retention.max_delta_bytes
        {
            if let Some((_, size)) = self.deltas.pop_front() {
                self.delta_bytes -= size;
            }
        }
        Ok(delta)
    }
}

pub fn active(status: &Status) -> bool {
    matches!(
        status,
        Status::Accepted | Status::Running | Status::Waiting | Status::CompletionPending
    )
}
fn pinned(record: &Record) -> bool {
    match &record.state {
        State::Activation { status }
        | State::Node { status, .. }
        | State::Request { status, .. }
        | State::MediaJob { status, .. }
        | State::Tool { status, .. } => active(status),
        _ => false,
    }
}

impl Snapshot {
    /// A dropped, duplicate or reordered delta never partially changes a client.
    pub fn apply(&mut self, delta: &Delta) -> Result<()> {
        if self.cursor != delta.base
            || delta.cursor.epoch != self.cursor.epoch
            || delta.cursor.number()?
                != delta.base.number()?.checked_add(1).ok_or(Error::Overflow)?
        {
            return Err(Error::ResyncRequired);
        }
        let mut records: BTreeMap<_, _> = self
            .records
            .iter()
            .map(|r| (r.id.clone(), r.clone()))
            .collect();
        for change in &delta.changes {
            match change {
                Change::Upsert { record } => {
                    records.insert(record.id.clone(), record.as_ref().clone());
                }
                Change::Remove { id } => {
                    records.remove(id);
                }
            }
        }
        self.records = records.into_values().collect();
        self.cursor = delta.cursor.clone();
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TimingSummary {
    pub count: u64,
    pub total_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u64")]
    pub minimum_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "u64")]
    pub maximum_ms: Option<u64>,
}
impl TimingSummary {
    pub fn observe(&mut self, duration_ms: u64) -> Result<()> {
        let next = Self {
            count: self.count.checked_add(1).ok_or(Error::Overflow)?,
            total_ms: self
                .total_ms
                .checked_add(duration_ms)
                .ok_or(Error::Overflow)?,
            minimum_ms: Some(self.minimum_ms.map_or(duration_ms, |n| n.min(duration_ms))),
            maximum_ms: Some(self.maximum_ms.map_or(duration_ms, |n| n.max(duration_ms))),
        };
        bytes(&next)?;
        *self = next;
        Ok(())
    }
}
