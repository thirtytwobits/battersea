//! Copyright (c) Scott A Dixon
//!
//! The failure every Battersea operation reports. See `contracts/failures.md`.
mod codes;
pub use codes::*;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A part of a failure that the host may withhold from a flow.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "state", content = "value", rename_all = "snake_case")]
pub enum Detail<T> {
    /// The reporter supplied nothing.
    #[default]
    Absent,
    Present(T),
    /// The reporter supplied a value and the host's disclosure removed it.
    Withheld,
}
impl<T> Detail<T> {
    pub fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }
    pub fn present(&self) -> Option<&T> {
        match self {
            Self::Present(value) => Some(value),
            Self::Absent | Self::Withheld => None,
        }
    }
    fn withhold(&mut self) {
        if matches!(self, Self::Present(_)) {
            *self = Self::Withheld;
        }
    }
}

/// What a host lets a flow receive of a failure.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FailureDisclosure {
    #[default]
    Full,
    /// Native detail, partial output and foreign error text are marked withheld.
    Withheld,
}

/// Who reported a failure.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Reporter {
    #[default]
    Battersea,
    Provider,
    Tool,
}

/// The operation a failure belongs to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Origin {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// What is known of the remote exchange a failure belongs to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Exchange {
    #[serde(default)]
    pub dispatch: DispatchState,
    #[serde(default)]
    pub attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    /// The identifier the provider issued for the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// The identifier the provider issued for the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
}

/// Output Battersea held when its operation failed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Partial {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<Value>,
}

/// What a failure wraps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    Failure(Failure),
    /// The text of an error of another type, then of each of its sources in order.
    Foreign(Detail<Vec<String>>),
}

/// One failed or incomplete operation. The parts are boxed so a `Result` that carries a
/// failure stays small.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, thiserror::Error)]
#[error("{}", .0.message)]
#[serde(transparent)]
pub struct Failure(Box<FailureParts>);
impl std::ops::Deref for Failure {
    type Target = FailureParts;
    fn deref(&self) -> &FailureParts {
        &self.0
    }
}
impl std::ops::DerefMut for Failure {
    fn deref_mut(&mut self) -> &mut FailureParts {
        &mut self.0
    }
}

/// The parts of a [`Failure`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FailureParts {
    #[schemars(with = "String")]
    pub kind: ErrorKind,
    /// Battersea's description, in fixed wording.
    pub message: String,
    #[serde(default)]
    pub reporter: Reporter,
    #[serde(default)]
    pub origin: Origin,
    #[serde(default)]
    pub exchange: Exchange,
    /// The limit an idle watchdog enforced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watchdog: Option<WatchdogFailure>,
    /// The tool runner's name for this refusal; `kind` is its class.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_code: Option<ToolCode>,
    /// Evidence about a nested tool call, independent from provider dispatch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<Box<ToolCallEvidence>>,
    /// The reporter's own account, whole.
    #[serde(default, skip_serializing_if = "Detail::is_absent")]
    pub native: Detail<Value>,
    #[serde(default, skip_serializing_if = "Detail::is_absent")]
    pub partial: Detail<Partial>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<Cause>,
}

impl Failure {
    /// A failure Battersea reports about `provider`.
    pub fn new(
        provider: impl Into<String>,
        message: impl Into<String>,
        kind: impl Into<ErrorKind>,
    ) -> Self {
        let mut failure = Self::local(message, kind);
        failure.origin.provider = Some(provider.into());
        failure
    }

    /// A failure Battersea reports outside any provider operation.
    pub fn local(message: impl Into<String>, kind: impl Into<ErrorKind>) -> Self {
        let kind = kind.into();
        let dispatch = if kind == ErrorKind::InvalidRequest {
            DispatchState::NotSent
        } else {
            DispatchState::Unknown
        };
        Self(Box::new(FailureParts {
            kind,
            message: message.into(),
            reporter: Reporter::Battersea,
            origin: Origin::default(),
            exchange: Exchange {
                dispatch,
                ..Exchange::default()
            },
            watchdog: None,
            tool_code: None,
            tool_call: None,
            native: Detail::Absent,
            partial: Detail::Absent,
            cause: None,
        }))
    }

    pub fn invalid_response(provider: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(provider, message, ErrorKind::InvalidResponse)
    }

    pub fn transport(provider: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(provider, message, ErrorKind::Transport)
    }

    /// Names a refusal of the tool runner.
    pub fn with_tool_code(mut self, code: ToolCode) -> Self {
        self.tool_code = Some(code);
        self
    }

    /// Retains evidence about a nested tool invocation.
    pub fn with_tool_call(mut self, evidence: ToolCallEvidence) -> Self {
        self.tool_call = Some(Box::new(evidence));
        self
    }

    pub fn with_dispatch(mut self, dispatch: DispatchState) -> Self {
        self.exchange.dispatch = dispatch;
        self
    }

    pub fn with_idle_watchdog(mut self, limit_ms: u64) -> Self {
        self.watchdog = Some(WatchdogFailure { limit_ms });
        self
    }

    pub fn with_status_code(mut self, status: u16) -> Self {
        self.exchange.status = Some(status);
        self
    }

    pub fn with_request_id(mut self, request_id: Option<String>) -> Self {
        self.exchange.request_id = request_id;
        self
    }

    /// Carries the reporter's own account, whole.
    pub fn with_native(mut self, reporter: Reporter, native: Value) -> Self {
        self.reporter = reporter;
        self.native = Detail::Present(native);
        self
    }

    pub fn with_partial(mut self, partial: Partial) -> Self {
        self.partial = Detail::Present(partial);
        self
    }

    /// Wraps another failure.
    pub fn with_cause(mut self, cause: Failure) -> Self {
        self.cause = Some(Cause::Failure(cause));
        self
    }

    /// Wraps an error of another type as its text and the text of each of its sources.
    pub fn caused_by(mut self, error: &(dyn std::error::Error + 'static)) -> Self {
        let mut texts = vec![error.to_string()];
        let mut source = error.source();
        while let Some(error) = source {
            texts.push(error.to_string());
            source = error.source();
        }
        self.cause = Some(Cause::Foreign(Detail::Present(texts)));
        self
    }

    /// Wraps the text of an error that does not implement `std::error::Error`.
    pub fn caused_by_text(mut self, text: impl Into<String>) -> Self {
        self.cause = Some(Cause::Foreign(Detail::Present(vec![text.into()])));
        self
    }

    /// The failure a flow receives under `disclosure`.
    pub fn disclose(&self, disclosure: FailureDisclosure) -> Self {
        let mut failure = self.clone();
        if disclosure == FailureDisclosure::Withheld {
            failure.withhold();
        }
        failure
    }

    fn withhold(&mut self) {
        self.native.withhold();
        self.partial.withhold();
        match self.cause.as_mut() {
            Some(Cause::Failure(cause)) => cause.withhold(),
            Some(Cause::Foreign(texts)) => texts.withhold(),
            None => {}
        }
    }

    /// Applies `visit` to every string a reporter or a lower layer could have written, at
    /// every depth of the cause chain.
    pub fn visit_text(&mut self, visit: &mut dyn FnMut(&mut String)) {
        visit(&mut self.message);
        if let ErrorKind::Other(kind) = &mut self.kind {
            let mut text = kind.to_string();
            visit(&mut text);
            *kind = text.into_boxed_str();
        }
        let parts = &mut *self.0;
        for text in [
            &mut parts.origin.operation,
            &mut parts.origin.provider,
            &mut parts.origin.backend,
            &mut parts.origin.model,
            &mut parts.exchange.request_id,
            &mut parts.exchange.response_id,
        ]
        .into_iter()
        .flatten()
        {
            visit(text);
        }
        if let Some(call) = self.tool_call.as_deref_mut() {
            visit(&mut call.call_id);
            visit(&mut call.tool_name);
            if let Some(id) = &mut call.request_id {
                visit(id);
            }
        }
        if let Detail::Present(native) = &mut self.native {
            visit_value(native, visit);
        }
        if let Detail::Present(partial) = &mut self.partial {
            visit(&mut partial.text);
            for block in &mut partial.blocks {
                visit_value(block, visit);
            }
        }
        match self.cause.as_mut() {
            Some(Cause::Failure(cause)) => cause.visit_text(visit),
            Some(Cause::Foreign(Detail::Present(texts))) => texts.iter_mut().for_each(visit),
            Some(Cause::Foreign(Detail::Absent | Detail::Withheld)) | None => {}
        }
    }
}

/// Applies `visit` to every string in `value`, object keys included.
fn visit_value(value: &mut Value, visit: &mut dyn FnMut(&mut String)) {
    match value {
        Value::String(text) => visit(text),
        Value::Array(items) => items.iter_mut().for_each(|item| visit_value(item, visit)),
        Value::Object(map) => {
            let entries = std::mem::take(map);
            for (mut key, mut item) in entries {
                visit(&mut key);
                visit_value(&mut item, visit);
                map.insert(key, item);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Debug, thiserror::Error)]
    #[error("outer")]
    struct Outer(#[source] Inner);
    #[derive(Debug, thiserror::Error)]
    #[error("inner")]
    struct Inner;

    fn nested() -> Failure {
        Failure::new("fixture", "Provider refused.", ErrorKind::Refusal)
            .with_native(Reporter::Provider, json!({"category": "secret-key"}))
            .with_partial(Partial {
                text: "partial secret-key".into(),
                blocks: vec![],
            })
            .with_cause(
                Failure::invalid_response("fixture", "Malformed event.")
                    .with_native(Reporter::Provider, json!("body"))
                    .caused_by(&Outer(Inner)),
            )
    }

    #[test]
    fn a_foreign_error_is_carried_as_its_text_and_its_sources_in_order() {
        let failure = Failure::local("Wrapped.", ErrorKind::Transport).caused_by(&Outer(Inner));
        assert_eq!(
            failure.cause.as_ref(),
            Some(&Cause::Foreign(Detail::Present(vec![
                "outer".into(),
                "inner".into()
            ])))
        );
    }

    #[test]
    fn withholding_marks_native_partial_and_foreign_text_at_every_depth() {
        let full = nested().disclose(FailureDisclosure::Full);
        assert_eq!(full, nested());
        let withheld = nested().disclose(FailureDisclosure::Withheld);
        assert_eq!(withheld.native, Detail::Withheld);
        assert_eq!(withheld.partial, Detail::Withheld);
        let Some(Cause::Failure(cause)) = withheld.cause.as_ref() else {
            panic!("cause is a failure")
        };
        assert_eq!(cause.native, Detail::Withheld);
        assert_eq!(
            cause.cause.as_ref(),
            Some(&Cause::Foreign(Detail::Withheld))
        );
        assert_eq!(cause.message, "Malformed event.");
        let text = serde_json::to_string(&withheld).unwrap();
        for hidden in ["secret-key", "body", "outer", "inner"] {
            assert!(!text.contains(hidden), "{hidden} was disclosed");
        }
    }

    #[test]
    fn an_absent_part_stays_absent_when_withheld() {
        let failure = Failure::local("Local.", ErrorKind::InvalidRequest)
            .disclose(FailureDisclosure::Withheld);
        assert_eq!(failure.native, Detail::Absent);
        assert_eq!(failure.partial, Detail::Absent);
    }

    #[test]
    fn every_string_at_every_depth_is_visited() {
        let mut failure = nested();
        failure.exchange.request_id = Some("req secret-key".into());
        failure.visit_text(&mut |text| *text = text.replace("secret-key", "[redacted]"));
        let text = serde_json::to_string(&failure).unwrap();
        assert!(!text.contains("secret-key"));
        assert_eq!(text.matches("[redacted]").count(), 3);
    }

    #[test]
    fn a_failure_survives_serialisation() {
        let failure = nested();
        let value = serde_json::to_value(&failure).unwrap();
        assert_eq!(serde_json::from_value::<Failure>(value).unwrap(), failure);
    }
}
