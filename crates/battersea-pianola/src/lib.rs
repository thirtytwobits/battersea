//! Copyright (c) Scott A Dixon
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// One Pianola roll: the whole program a player piano reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    deny_unknown_fields,
    bound(deserialize = "S: Deserialize<'de>, A: Deserialize<'de>, E: Deserialize<'de>")
)]
pub struct Roll<S, A, E> {
    /// Roll schema version. The runner refuses a version it does not
    /// understand.
    pub version: u32,
    /// Human-readable identifier (shown in reports).
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defaults: Option<RollDefaults>,
    pub setup: S,
    pub steps: Vec<RollStep<A, E>>,
    /// Optional lossless port capture ("tee"): mirror named port
    /// emissions to a recording file as the roll plays. Absent = no
    /// capture (unit rolls are unaffected).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tee: Option<RollTee>,
    /// Optional criteria-based grading: which strike errors are excused
    /// (not failures) and what gates the roll. Absent = legacy behaviour
    /// (every expectation hard; the gate is "no failures").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criteria: Option<RollCriteria>,
}

/// Criteria-based grading configuration. Turns a roll's result from a
/// red/green light into a graded report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollCriteria {
    /// Strike-error classes that are EXCUSED — recorded as `excused`,
    /// never as a failure. The "not-failure" conditions from the spec:
    /// no-connectivity, spend-limit, throttling, provider 5xx. Each entry
    /// is a built-in class name or a custom matcher.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_errors: Vec<AllowedError>,
    /// The per-roll gate policy. Absent = default (`hard_pass`, no grade
    /// threshold).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success: Option<RollSuccessPolicy>,
}

/// An excused strike-error class: a built-in name or a custom matcher.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AllowedError {
    /// Built-in class: `connectivity` | `rate_limit` | `spend_limit` |
    /// `server_error`.
    Class(String),
    /// Custom match on the engine error's structured `code` and/or its
    /// message (regex). Either field present is a match condition; both
    /// present means both must match.
    Custom {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_regex: Option<String>,
    },
}

/// The per-roll gate policy: what makes the run pass (exit 0).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollSuccessPolicy {
    /// Gate requirement. Currently only `hard_pass` (no hard failures);
    /// named so the policy can grow new modes. Default `hard_pass`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require: Option<String>,
    /// Optional minimum aggregate grade (0.0–1.0) the run must also meet.
    /// When set, a run with graded criteria below this fails the gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_grade: Option<f64>,
}

/// Lossless port-capture configuration for a run. Declares which ports
/// to mirror and where the recording is written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollTee {
    /// Recording path, resolved by the CLI relative to the roll file.
    /// Written as NDJSON — one captured emission per line.
    pub output: String,
    /// Ports to capture. Only these are recorded (opt-in per port).
    pub ports: Vec<RollTeePort>,
}

/// One teed port: a port name, optionally scoped to a node via the same
/// symbolic selector `activate` uses (so it survives node-id churn).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollTeePort {
    pub port: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<NodeSelector>,
}

/// Fallback values applied to every step that omits them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollDefaults {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_binding: Option<String>,
}

/// One program step: strike one input, then evaluate its expectations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    deny_unknown_fields,
    bound(deserialize = "A: Deserialize<'de>, E: Deserialize<'de>")
)]
pub struct RollStep<A, E> {
    /// Unique, stable step id. Referenced by `order` assertions and
    /// quoted in failure reports.
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// The input to strike. `do` is a Rust keyword, hence the rename.
    #[serde(rename = "do")]
    pub action: A,
    #[serde(default)]
    pub expect: Vec<E>,
}

/// Symbolic node selector — resolved against the bound flow at run
/// time so a roll survives node-id churn. Matches the node whose
/// `definition_name == definition`; `instance` disambiguates when the
/// flow has several of that definition. The schema requires exactly
/// one of `node_id` / `node` on an `activate` action.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeSelector {
    pub definition: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
}

/// How an assertion's result is folded into the graded report.
///
/// - `hard` (default): a violation is a hard FAILURE that fails the
///   roll's gate. The fundamental-rule checks (e.g. "the parser could
///   separate user-facing text from out-of-story").
/// - `graded`: a violation lowers the aggregate GRADE but does not, on
///   its own, fail the gate ("pass-with-poor-grade" — e.g. a parsed
///   response missing optional data). Contributes `weight` to the grade.
/// - `advisory`: recorded for the report but never gates and never
///   grades — pure observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    #[default]
    Hard,
    Graded,
    Advisory,
}

/// What an `agent_grade` criterion grades — a captured output port's
/// value for the step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentGradeSubject {
    pub port: String,
}

/// Who should grade an `agent_grade` criterion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentGradeGrader {
    /// Grader bridge identifier registered by the host.
    pub kind: String,
    /// Optional model selector passed to the grader bridge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Optional human label for reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Exactly one matcher against a text value. Externally tagged so the
/// JSON shape is a single-key object (`{equals: "..."}`,
/// `{present: true}`) — 1:1 with the schema's oneOf-of-single-key
/// objects, and serde rejects unknown / multiple keys for free.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextMatch {
    Equals(String),
    Contains(String),
    Regex(String),
    /// The port/turn must NOT have emitted at all.
    Absent(bool),
    /// The port/turn must have emitted (any value).
    Present(bool),
}

/// Required `ChatToolCallCapture` status for a `tool_call` assertion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatusMatch {
    Succeeded,
    Failed,
    Cancelled,
}

/// References an observable event for an `order` assertion: a port
/// emission or a tool call, scoped to a step id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventRef {
    pub step: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
}

/// One teed port target. Captures emissions on `port`, optionally
/// scoped to a resolved `node_id` (a `tee.ports` entry with a `node`
/// selector); `None` matches that port on any node.
#[derive(Debug, Clone)]
pub struct PianolaTapTarget {
    pub flow_key: String,
    pub node_id: Option<String>,
    pub port: String,
}

/// One captured port emission — the FULL value, never truncated.
#[derive(Debug, Clone)]
pub struct TappedEmission {
    pub node_id: String,
    pub port: String,
    pub token_type: String,
    pub value: Value,
    pub at: DateTime<Utc>,
    pub ordinal: u64,
}

/// The lossless capture buffer a roll's runner installs on its sandbox
/// engine for the duration of a run. Capture is ordered and byte-bounded;
/// exceeding capacity records a sticky failure instead of truncating a value.
#[derive(Debug)]
pub struct PianolaTap {
    targets: Vec<PianolaTapTarget>,
    emissions: Vec<TappedEmission>,
    next_ordinal: u64,
    remaining_bytes: usize,
    error: Option<String>,
}

/// One captured port emission attributed to a roll step. Serialised one
/// per NDJSON line into the roll's `tee.output`.
#[derive(Debug, Clone, PartialEq)]
pub struct RollRecordingEntry {
    pub step_id: String,
    pub node_id: String,
    pub port: String,
    pub token_type: String,
    pub value: Value,
    pub timestamp: String,
    pub ordinal: u64,
}

/// Outcome of playing one roll. The report is always returned (even on
/// failure) so callers can render a full transcript; only
/// infrastructural problems (an unresolvable flow, a session that won't
/// create) surface as `Err`.
#[derive(Debug, Clone, PartialEq)]
pub struct RollReport {
    pub roll_name: String,
    /// Whether every criterion literally held.
    pub all_criteria_held: bool,
    /// The roll's GATE verdict (per `criteria.success`; default: no hard
    /// failures). This is the CI exit-code signal.
    pub required_criteria_held: bool,
    pub steps: Vec<StepReport>,
    /// Aggregate grade in `0.0..=1.0` (weighted graded-criteria score)
    /// when the roll has any resolved graded criterion; `None` otherwise.
    pub grade: Option<f64>,
    /// The roll's effective `criteria.success.min_grade`, echoed for the
    /// grader harness's gate recomputation.
    pub min_grade: Option<f64>,
    /// Whether any criterion is a pending `agent_grade`.
    pub pending_grades: bool,
    /// Lossless port recording, in capture order, when the roll declares
    /// a `tee`. Empty otherwise.
    pub recording: Vec<RollRecordingEntry>,
    /// The roll's `tee.output` path (verbatim), so the CLI can resolve
    /// it relative to the roll file and write the NDJSON. `None` when the
    /// roll declares no `tee`.
    pub recording_output: Option<String>,
}

/// Outcome of one step: the strike plus every expectation it declared.
#[derive(Debug, Clone, PartialEq)]
pub struct StepReport {
    pub step_id: String,
    /// Strike succeeded AND every assertion literally held.
    pub all_criteria_held: bool,
    /// Every required criterion held. Graded and advisory misses do not
    /// make this false.
    pub required_criteria_held: bool,
    /// The matched `allowed_errors` class when a strike error is excused.
    pub error_class: Option<String>,
    /// Set when the strike itself errored; when present, expectations
    /// were not evaluated.
    pub strike_error: Option<String>,
    pub expectations: Vec<ExpectationReport>,
}

/// Outcome of one assertion within a step.
#[derive(Debug, Clone, PartialEq)]
pub struct ExpectationReport {
    /// Human-readable restatement of the assertion (for transcripts).
    pub summary: String,
    /// Whether the underlying assertion literally held.
    pub all_criteria_held: bool,
    /// Whether this assertion's required criterion held. Graded and
    /// advisory misses do not make this false.
    pub required_criteria_held: bool,
    /// `agent_grade` only: requested grader identity.
    pub grader: Option<AgentGradeGrader>,
    /// `agent_grade` only: whether the grader model must differ from
    /// the model that produced the subject under test.
    pub require_distinct_model: bool,
    /// `agent_grade` only: backend id that produced the subject under test.
    pub subject_backend: Option<String>,
    /// `agent_grade` only: provider model that produced the subject under test.
    pub subject_model: Option<String>,
    pub severity: Severity,
    /// Graded-criterion score / max (`max` present for any graded
    /// criterion; `score` is `None` while an `agent_grade` is pending).
    pub score: Option<f64>,
    pub max: Option<f64>,
    /// Why it failed (None when it passed); also surfaced as graded
    /// notes.
    pub detail: Option<String>,
    /// `agent_grade` only: awaiting an out-of-band score from the grader
    /// harness. `score` is `None` until filled.
    pub pending: bool,
    /// `agent_grade` only: the grading instruction for the grader agent.
    pub rubric: Option<String>,
    /// `agent_grade` only: the captured subject text to grade.
    pub subject: Option<String>,
}

/// One observed port emission, in engine-recorded time.
#[derive(Debug, Clone)]
pub struct PortEmission {
    pub port: String,
    pub value: Value,
    pub at: Option<DateTime<Utc>>,
}

/// One observed tool call, in engine-recorded time.
#[derive(Debug, Clone)]
pub struct ToolObservation {
    pub name: String,
    pub status: String,
    pub arguments_json: String,
    pub at: Option<DateTime<Utc>>,
}

/// The raw result of running one assertion, before severity is folded
/// in. `evaluate_one` produces this; `classify_assertion` turns it into
/// the graded [`ExpectationReport`].
pub struct RawAssertion {
    pub summary: String,
    pub passed: bool,
    pub detail: Option<String>,
}

pub fn value_as_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

pub fn text_match_passes(matcher: &TextMatch, observed: Option<&str>) -> Result<bool, String> {
    Ok(match matcher {
        TextMatch::Equals(expected) => observed == Some(expected.as_str()),
        TextMatch::Contains(needle) => observed.map(|t| t.contains(needle)).unwrap_or(false),
        TextMatch::Regex(pattern) => {
            let re = regex_lite::Regex::new(pattern)
                .map_err(|error| format!("invalid regex {pattern:?}: {error}"))?;
            observed.map(|t| re.is_match(t)).unwrap_or(false)
        }
        TextMatch::Absent(_) => observed.is_none(),
        TextMatch::Present(_) => observed.is_some(),
    })
}

/// Returns whether `subset` is a deep-partial of `superset` (every key
/// in `subset` present and equal in `superset`; recurses into objects).
pub fn json_subset(subset: &Value, superset: &Value) -> bool {
    match (subset, superset) {
        (Value::Object(sub), Value::Object(sup)) => sub.iter().all(|(key, sub_value)| {
            sup.get(key)
                .map(|sup_value| json_subset(sub_value, sup_value))
                .unwrap_or(false)
        }),
        _ => subset == superset,
    }
}

/// Aggregate grade in `0.0..=1.0`: total graded score over total graded
/// max across every expectation. `None` when the roll has no graded
/// criteria.
pub fn aggregate_grade(steps: &[StepReport]) -> Option<f64> {
    let (score, max) = steps
        .iter()
        .flat_map(|step| &step.expectations)
        .filter_map(|expectation| match (expectation.score, expectation.max) {
            (Some(score), Some(max)) => Some((score, max)),
            _ => None,
        })
        .fold(
            (0.0_f64, 0.0_f64),
            |(score, max), (entry_score, entry_max)| (score + entry_score, max + entry_max),
        );
    (max > 0.0).then_some(score / max)
}

/// Computes the roll's GATE verdict from the per-roll success policy.
/// Default: no hard failures anywhere (excused/graded/advisory never
/// gate). An optional `min_grade` additionally requires the aggregate
/// grade to meet the threshold (a run with `min_grade` set but no graded
/// criteria fails — the policy asked for a grade that doesn't exist).
pub fn compute_gate(
    steps: &[StepReport],
    grade: Option<f64>,
    success: Option<&RollSuccessPolicy>,
) -> bool {
    if steps.iter().any(|step| !step.required_criteria_held) {
        return false;
    }
    if let Some(min_grade) = success.and_then(|policy| policy.min_grade) {
        match grade {
            Some(value) => return value + 1e-9 >= min_grade,
            None => return false,
        }
    }
    true
}

/// First non-empty line of a rubric, clamped, for transcript summaries.
pub fn rubric_excerpt(rubric: &str) -> String {
    let line = rubric
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    if line.chars().count() > 80 {
        format!("{}…", line.chars().take(80).collect::<String>())
    } else {
        line.to_string()
    }
}

/// Folds an assertion result into literal and required criterion truth.
/// Graded and advisory misses do not fail required criteria here; grade
/// floors are applied at the roll gate.
pub fn classify_assertion(raw: RawAssertion, severity: Severity, weight: f64) -> ExpectationReport {
    let (required_criteria_held, score, max) = match severity {
        Severity::Hard => (raw.passed, None, None),
        Severity::Graded => (
            true,
            Some(if raw.passed { weight } else { 0.0 }),
            Some(weight),
        ),
        Severity::Advisory => (true, None, None),
    };
    ExpectationReport {
        summary: raw.summary,
        all_criteria_held: raw.passed,
        required_criteria_held,
        grader: None,
        require_distinct_model: false,
        subject_backend: None,
        subject_model: None,
        severity,
        score,
        max,
        detail: raw.detail,
        pending: false,
        rubric: None,
        subject: None,
    }
}

pub fn pass(summary: String) -> RawAssertion {
    RawAssertion {
        summary,
        passed: true,
        detail: None,
    }
}

pub fn fail(summary: String, detail: String) -> RawAssertion {
    RawAssertion {
        summary,
        passed: false,
        detail: Some(detail),
    }
}

/// Wire string for a severity (`hard` | `graded` | `advisory`).
pub fn severity_str(severity: Severity) -> &'static str {
    match severity {
        Severity::Hard => "hard",
        Severity::Graded => "graded",
        Severity::Advisory => "advisory",
    }
}

impl PianolaTap {
    pub fn new(targets: Vec<PianolaTapTarget>, max_bytes: usize) -> Self {
        Self {
            targets,
            emissions: Vec::new(),
            next_ordinal: 0,
            remaining_bytes: max_bytes,
            error: None,
        }
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Captures one emission iff a target matches its `port` (and
    /// `node_id`, when the target scopes one). Called from
    /// `emit_flow_token` for every emission; a no-op when no target
    /// matches, so untenanted ports cost only the match scan.
    pub fn capture(
        &mut self,
        flow_key: &str,
        node_id: &str,
        port: &str,
        token_type: &str,
        value: &Value,
    ) {
        let matched = self.targets.iter().any(|target| {
            target.flow_key == flow_key
                && target.port == port
                && target.node_id.as_deref().is_none_or(|n| n == node_id)
        });
        if !matched || self.error.is_some() {
            return;
        }
        struct Capacity(usize);
        impl std::io::Write for Capacity {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 = self.0.checked_sub(bytes.len()).ok_or_else(|| {
                    std::io::Error::other("Pianola capture exceeded its byte limit")
                })?;
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut capacity = Capacity(
            self.remaining_bytes
                .saturating_sub(std::mem::size_of::<TappedEmission>()),
        );
        if let Err(error) =
            serde_json::to_writer(&mut capacity, &(node_id, port, token_type, value))
        {
            self.error = Some(error.to_string());
            return;
        }
        self.remaining_bytes = capacity.0;
        let ordinal = self.next_ordinal;
        self.next_ordinal += 1;
        self.emissions.push(TappedEmission {
            node_id: node_id.to_string(),
            port: port.to_string(),
            token_type: token_type.to_string(),
            value: value.clone(),
            at: Utc::now(),
            ordinal,
        });
    }

    /// Number of emissions captured so far (the runner snapshots this
    /// around each strike to attribute emissions to a step).
    pub fn captured_len(&self) -> usize {
        self.emissions.len()
    }

    /// Clones the emissions captured at or after `start` — the runner's
    /// per-step window: snapshot `captured_len()` before a strike, then
    /// pull everything from that index after it.
    pub fn emissions_from(&self, start: usize) -> Vec<TappedEmission> {
        self.emissions
            .get(start..)
            .map(<[_]>::to_vec)
            .unwrap_or_default()
    }
}

fn default_occurrences() -> u32 {
    1
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GenericExpectation {
    Port {
        port: String,
        #[serde(rename = "match")]
        matcher: TextMatch,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        within_ms: Option<u64>,
        #[serde(default)]
        severity: Severity,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        weight: Option<u32>,
    },
    ToolCall {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        arguments_match: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ToolCallStatusMatch>,
        #[serde(default = "default_occurrences")]
        occurrences: u32,
        #[serde(default)]
        severity: Severity,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        weight: Option<u32>,
    },
    Order {
        before: EventRef,
        after: EventRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default)]
        severity: Severity,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        weight: Option<u32>,
    },
    Count {
        target: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<Value>,
        n: u32,
        #[serde(default)]
        severity: Severity,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        weight: Option<u32>,
    },
    AgentGrade {
        /// The grading instruction handed to the grader agent.
        rubric: String,
        /// What to grade. Defaults to the `user_response` port's
        /// captured value for the step.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subject: Option<AgentGradeSubject>,
        /// Which grader bridge/model should judge this criterion.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        grader: Option<AgentGradeGrader>,
        /// Require the grader model to differ from the model that
        /// produced the subject under test.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        require_distinct_model: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        weight: Option<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Expectation<D> {
    Generic(GenericExpectation),
    Domain(D),
}
pub trait DomainCriterion {
    fn kind(&self) -> &str;
    fn severity(&self) -> Severity;
    fn weight(&self) -> f64;
}
impl<D: DomainCriterion> Expectation<D> {
    pub fn severity(&self) -> Severity {
        match self {
            Self::Generic(e) => e.severity(),
            Self::Domain(e) => e.severity(),
        }
    }
    pub fn weight(&self) -> f64 {
        match self {
            Self::Generic(e) => e.weight(),
            Self::Domain(e) => e.weight(),
        }
    }
}
impl GenericExpectation {
    pub fn severity(&self) -> Severity {
        match self {
            Self::Port { severity, .. }
            | Self::ToolCall { severity, .. }
            | Self::Order { severity, .. }
            | Self::Count { severity, .. } => *severity,
            Self::AgentGrade { .. } => Severity::Graded,
        }
    }
    pub fn weight(&self) -> f64 {
        match self {
            Self::Port { weight, .. }
            | Self::ToolCall { weight, .. }
            | Self::Order { weight, .. }
            | Self::Count { weight, .. }
            | Self::AgentGrade { weight, .. } => weight.unwrap_or(1).max(1) as f64,
        }
    }
}
pub trait Observation {
    fn ports(&self) -> &[PortEmission];
    fn tools(&self) -> &[ToolObservation];
    fn strike_ms(&self) -> u128;
    fn count(&self, target: &str, filter: Option<&Value>) -> Option<u32>;
}
#[derive(Debug, Clone, Default)]
pub struct Timeline {
    pub ports: Vec<PortEmission>,
    pub tools: Vec<ToolObservation>,
    pub strike_ms: u128,
}
impl Observation for Timeline {
    fn ports(&self) -> &[PortEmission] {
        &self.ports
    }
    fn tools(&self) -> &[ToolObservation] {
        &self.tools
    }
    fn strike_ms(&self) -> u128 {
        self.strike_ms
    }
    fn count(&self, target: &str, filter: Option<&Value>) -> Option<u32> {
        match target {
            "tool_call" => Some(count_tools(&self.tools, filter)),
            "port" => Some(
                self.ports
                    .iter()
                    .filter(|p| {
                        filter
                            .and_then(|f| f.get("port"))
                            .and_then(Value::as_str)
                            .is_none_or(|name| name == p.port)
                    })
                    .count() as u32,
            ),
            _ => None,
        }
    }
}
pub fn count_tools(tools: &[ToolObservation], filter: Option<&Value>) -> u32 {
    tools
        .iter()
        .filter(|tool| {
            filter
                .and_then(|f| f.get("name"))
                .and_then(Value::as_str)
                .is_none_or(|want| tool.name == want || tool.name == want.replace('.', "_"))
        })
        .count() as u32
}
pub struct AssertionRegistry<D, C> {
    handlers: HashMap<String, fn(&D, &C) -> RawAssertion>,
}
impl<D, C> Default for AssertionRegistry<D, C> {
    fn default() -> Self {
        Self {
            handlers: HashMap::new(),
        }
    }
}
impl<D: DomainCriterion, C: Observation> AssertionRegistry<D, C> {
    pub fn register(
        &mut self,
        kind: &str,
        handler: fn(&D, &C) -> RawAssertion,
    ) -> Result<(), String> {
        if kind.trim().is_empty() || self.handlers.contains_key(kind) {
            return Err(format!("duplicate or blank assertion kind: {kind}"));
        }
        self.handlers.insert(kind.to_owned(), handler);
        Ok(())
    }
    pub fn evaluate(
        &self,
        step_id: &str,
        expectation: &Expectation<D>,
        timeline: &C,
        prior: &HashMap<String, C>,
        default_grade_port: &str,
    ) -> ExpectationReport {
        match expectation {
            Expectation::Generic(GenericExpectation::AgentGrade {
                rubric,
                subject,
                grader,
                require_distinct_model,
                ..
            }) => {
                let port = subject
                    .as_ref()
                    .map(|s| s.port.as_str())
                    .unwrap_or(default_grade_port);
                ExpectationReport {
                    summary: format!("agent_grade `{port}`: {}", rubric_excerpt(rubric)),
                    all_criteria_held: false,
                    required_criteria_held: true,
                    grader: grader.clone(),
                    require_distinct_model: *require_distinct_model,
                    subject_backend: None,
                    subject_model: None,
                    severity: Severity::Graded,
                    score: None,
                    max: Some(expectation.weight()),
                    detail: None,
                    pending: true,
                    rubric: Some(rubric.clone()),
                    subject: timeline
                        .ports()
                        .iter()
                        .find(|p| p.port == port)
                        .map(|p| value_as_text(&p.value)),
                }
            }
            _ => {
                let raw = match expectation {
                    Expectation::Generic(e) => evaluate_generic(step_id, e, timeline, prior),
                    Expectation::Domain(e) => self
                        .handlers
                        .get(e.kind())
                        .map(|handler| handler(e, timeline))
                        .unwrap_or_else(|| {
                            fail(e.kind().into(), "unregistered domain assertion".into())
                        }),
                };
                classify_assertion(raw, expectation.severity(), expectation.weight())
            }
        }
    }
}
pub fn evaluate_generic<C: Observation>(
    step_id: &str,
    expectation: &GenericExpectation,
    timeline: &C,
    prior: &HashMap<String, C>,
) -> RawAssertion {
    match expectation {
        GenericExpectation::Port {
            port,
            matcher,
            within_ms,
            ..
        } => {
            let summary = format!("port `{port}` {matcher:?}");
            let emission = timeline
                .ports()
                .iter()
                .find(|emission| &emission.port == port);
            let observed_text = emission
                .as_ref()
                .map(|emission| value_as_text(&emission.value));
            match text_match_passes(matcher, observed_text.as_deref()) {
                Err(error) => return fail(summary, error),
                Ok(false) => {
                    return fail(
                        summary,
                        format!(
                            "no match; observed {:?}",
                            observed_text.unwrap_or_else(|| "<no emission>".to_string())
                        ),
                    )
                }
                Ok(true) => {}
            }
            if let Some(bound) = within_ms {
                if emission.is_some() && timeline.strike_ms() > u128::from(*bound) {
                    return fail(
                        summary,
                        format!(
                            "arrived but strike took {}ms > within_ms {bound}",
                            timeline.strike_ms()
                        ),
                    );
                }
            }
            pass(summary)
        }
        GenericExpectation::ToolCall {
            name,
            arguments_match,
            status,
            occurrences,
            ..
        } => {
            let summary = format!("tool_call `{name}` x{occurrences}");
            let provider = name.replace('.', "_");
            let hits = timeline
                .tools()
                .iter()
                .filter(|tool| tool.name == provider || tool.name == *name)
                .filter(|tool| {
                    status
                        .map(|want| {
                            tool.status
                                == match want {
                                    ToolCallStatusMatch::Succeeded => "succeeded",
                                    ToolCallStatusMatch::Failed => "failed",
                                    ToolCallStatusMatch::Cancelled => "cancelled",
                                }
                        })
                        .unwrap_or(true)
                })
                .filter(|tool| {
                    let Some(want) = arguments_match else {
                        return true;
                    };
                    serde_json::from_str::<Value>(&tool.arguments_json)
                        .map(|got| json_subset(want, &got))
                        .unwrap_or(false)
                })
                .count() as u32;
            if hits == *occurrences {
                pass(summary)
            } else {
                fail(summary, format!("matched {hits}, expected {occurrences}"))
            }
        }
        GenericExpectation::Order {
            before,
            after,
            description,
            ..
        } => {
            let summary = description
                .clone()
                .unwrap_or_else(|| format!("order {before:?} before {after:?}"));
            let resolve = |reference: &EventRef| -> Option<DateTime<Utc>> {
                let tl = if reference.step == step_id {
                    Some(timeline)
                } else {
                    prior.get(&reference.step)
                };
                let tl = tl?;
                if let Some(port) = &reference.port {
                    tl.ports()
                        .iter()
                        .find(|emission| &emission.port == port)
                        .and_then(|emission| emission.at)
                } else if let Some(tool) = &reference.tool {
                    let provider = tool.replace('.', "_");
                    tl.tools()
                        .iter()
                        .find(|observed| observed.name == provider || &observed.name == tool)
                        .and_then(|observed| observed.at)
                } else {
                    None
                }
            };
            match (resolve(before), resolve(after)) {
                (Some(b), Some(a)) if b <= a => pass(summary),
                (Some(b), Some(a)) => fail(summary, format!("before={b} is not <= after={a}")),
                _ => fail(
                    summary,
                    "could not locate one or both ordered events in the timeline".to_string(),
                ),
            }
        }
        GenericExpectation::Count {
            target, filter, n, ..
        } => {
            let summary = format!("count {target:?} == {n}");
            match timeline.count(target, filter.as_ref()) {
                Some(actual) if actual == *n => pass(summary),
                Some(actual) => fail(summary, format!("observed {actual}, expected {n}")),
                None => fail(summary, format!("unregistered count target: {target}")),
            }
        }
        GenericExpectation::AgentGrade { .. } => {
            fail("agent_grade".into(), "requires the grading hook".into())
        }
    }
}
/// Resolve a pending grade after a host grader returns a normalised score and notes.
/// Invalid scores or grader identities leave the pending report unchanged.
pub fn resolve_grade(
    report: &mut ExpectationReport,
    score: f64,
    notes: String,
    grader_model: Option<&str>,
) -> Result<(), String> {
    if !report.pending || !score.is_finite() || !(0.0..=1.0).contains(&score) {
        return Err("expected a pending grade and a finite score in 0..=1".into());
    }
    if report.require_distinct_model
        && (grader_model.is_none()
            || report.subject_model.as_deref().is_none()
            || grader_model == report.subject_model.as_deref())
    {
        return Err("grading requires a distinct, identified subject and grader model".into());
    }
    let weight = report.max.ok_or("grade has no weight")?;
    report.score = Some(score * weight);
    report.detail = Some(notes);
    report.pending = false;
    report.all_criteria_held = score >= 1.0;
    Ok(())
}

pub mod schema;
