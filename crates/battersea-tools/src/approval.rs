//! Copyright (c) Scott A Dixon
//! Host decisions applied to validated calls in an immutable run scope.
use crate::{registry::refusal, Tool, ToolRegistry};
use async_trait::async_trait;
use battersea_model::{
    configuration_digest, EngineAdapterRequestError, EngineLocalToolCall,
    EngineLocalToolDefinition, EngineLocalToolExecutor, EngineLocalToolResult, ErrorKind,
    ToolCallOutcome, ToolCode,
};
use serde_json::json;
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::Instant,
};

/// Host identity and absolute deadline for one tool conversation.
#[derive(Clone, Debug)]
pub struct ToolApprovalScope {
    pub run_id: String,
    /// Host revision covering the registry's selection, [`ToolRegistry::revision`], its grant
    /// and its credential scopes.
    pub binding_revision: String,
    pub deadline: Instant,
}

/// An immutable, schema-validated request presented to the host policy.
///
/// `revision` binds the run, binding revision, complete tool record and call. Hosts that
/// retain decisions must key them by this revision. Tool annotations are author claims.
pub struct ToolApprovalRequest<'a> {
    pub scope: &'a ToolApprovalScope,
    pub tool: &'a Tool,
    pub call: &'a EngineLocalToolCall,
    pub revision: &'a str,
}

/// The host's decision for the exact request passed to its policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolApprovalDecision {
    Allow,
    Deny,
    /// Stop before dispatch so the host can obtain a decision.
    Pending,
}

/// Evaluates an already available host decision without waiting for user interaction.
///
/// Policies must return promptly; the synchronous callback must not wait for I/O.
/// Return `Pending` when a decision requires interaction. Pending stops the provider
/// conversation with `ToolApprovalPending`; durable continuation is host-owned.
pub trait ToolApprovalPolicy: Send + Sync {
    fn decide(&self, request: ToolApprovalRequest<'_>) -> ToolApprovalDecision;
}

/// A registry gated by one host policy and run identity.
///
/// Clones share call-ID reservations. Every valid logical call ID is reserved before policy
/// evaluation and remains reserved after denial, pending, cancellation or failure. Reusing
/// an ID is refused, including with altered arguments. Creating another executor does not
/// restore these reservations; hosts must reconcile interrupted runs before replacement.
#[derive(Clone)]
pub struct ApprovedToolExecutor {
    registry: ToolRegistry,
    scope: Arc<ToolApprovalScope>,
    policy: Arc<dyn ToolApprovalPolicy>,
    calls: Arc<Mutex<BTreeSet<String>>>,
    max_calls: usize,
}

impl ApprovedToolExecutor {
    /// Pins the registry and policy for a run. `max_calls` must be in 1..=65536.
    ///
    /// Apply profile selection before construction. Call-ID digests occupy 64 bytes each;
    /// reservations are retained for the executor's lifetime and are never evicted.
    /// Execute calls inside a Tokio runtime with its time driver enabled.
    pub fn new(
        registry: ToolRegistry,
        scope: ToolApprovalScope,
        policy: Arc<dyn ToolApprovalPolicy>,
        max_calls: usize,
    ) -> Result<Self, EngineAdapterRequestError> {
        if scope.run_id.trim().is_empty()
            || scope.binding_revision.trim().is_empty()
            || !(1..=65536).contains(&max_calls)
        {
            return Err(refusal(
                "Invalid tool approval scope or capacity.",
                ToolCode::InvalidToolApproval,
            ));
        }
        Ok(Self {
            registry,
            scope: Arc::new(scope),
            policy,
            calls: Arc::new(Mutex::new(BTreeSet::new())),
            max_calls,
        })
    }

    /// Definitions from the registry snapshot used for approval and dispatch.
    pub fn definitions(&self) -> Vec<EngineLocalToolDefinition> {
        self.registry.definitions()
    }
}

#[allow(clippy::double_must_use)]
#[async_trait]
impl EngineLocalToolExecutor for ApprovedToolExecutor {
    async fn call(
        &self,
        call: EngineLocalToolCall,
    ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
        let prepared = self.registry.prepare(call)?;
        let refused = |code, message: &str, kind| {
            EngineAdapterRequestError::new("tools", message, kind)
                .with_tool_code(code)
                .with_tool_call(prepared.evidence.clone())
        };
        if prepared.call.id.trim().is_empty() {
            return Err(refused(
                ToolCode::InvalidToolCallId,
                "Tool call identity is blank.",
                ErrorKind::InvalidRequest,
            ));
        }
        if Instant::now() >= self.scope.deadline {
            return Err(refused(
                ToolCode::ToolApprovalExpired,
                "Tool approval deadline expired.",
                ErrorKind::Expired,
            ));
        }
        battersea_model::payload::check_payload(
            &(
                &self.scope.run_id,
                &self.scope.binding_revision,
                prepared.tool(),
                &prepared.call,
            ),
            "tools",
        )
        .map_err(|error| {
            error
                .with_tool_code(ToolCode::ToolTranscriptTooLarge)
                .with_tool_call(prepared.evidence.clone())
        })?;
        let call_id = configuration_digest(&json!(prepared.call.id));
        {
            let mut calls = self.calls.lock().unwrap();
            if calls.contains(&call_id) {
                return Err(refused(
                    ToolCode::InvalidToolCallId,
                    "Tool call identity has already been considered.",
                    ErrorKind::InvalidRequest,
                ));
            }
            if calls.len() == self.max_calls {
                return Err(refused(
                    ToolCode::ToolApprovalCapacity,
                    "Tool approval capacity exhausted.",
                    ErrorKind::InvalidRequest,
                ));
            }
            calls.insert(call_id);
        }
        let revision = configuration_digest(&json!({
            "run": self.scope.run_id, "binding": self.scope.binding_revision,
            "tool": prepared.tool(), "call": prepared.call,
        }));
        let decision = self.policy.decide(ToolApprovalRequest {
            scope: &self.scope,
            tool: prepared.tool(),
            call: &prepared.call,
            revision: &revision,
        });
        if Instant::now() >= self.scope.deadline {
            return Err(refused(
                ToolCode::ToolApprovalExpired,
                "Tool approval deadline expired.",
                ErrorKind::Expired,
            ));
        }
        match decision {
            ToolApprovalDecision::Deny => {
                return Err(refused(
                    ToolCode::ToolApprovalDenied,
                    "Host denied the tool call.",
                    ErrorKind::Permission,
                ))
            }
            ToolApprovalDecision::Pending => {
                return Err(refused(
                    ToolCode::ToolApprovalPending,
                    "Tool call awaits a host decision.",
                    ErrorKind::Permission,
                ))
            }
            ToolApprovalDecision::Allow => (),
        }
        let mut evidence = prepared.evidence.clone();
        evidence.outcome = ToolCallOutcome::Uncertain;
        tokio::time::timeout_at(self.scope.deadline.into(), prepared.execute())
            .await
            .map_err(|_| {
                EngineAdapterRequestError::new(
                    "tools",
                    "Tool execution deadline expired.",
                    ErrorKind::Timeout,
                )
                .with_tool_call(evidence)
            })?
    }
}
