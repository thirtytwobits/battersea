//! Copyright (c) Scott A Dixon
//! What the application supplies: its flows, runs, approval journals and MCP connections.
// async-trait adds must_use to boxed futures, which already carry it.
#![allow(clippy::double_must_use)]
use async_trait::async_trait;
use battersea_flow::{FlowDocument, FlowValidationResult};
use battersea_tools::DurableToolExecutor;
use clapham::{OAuthContext, RecordSink, Refusal};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::Arc;

/// One stored flow, readable or not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowSummary {
    pub flow_key: String,
    pub title: String,
    /// Why the flow cannot be read, such as an unsupported document version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub unavailable_reason: Option<String>,
}

/// The explicit format upgrade of one stored flow, fenced on the revision it upgrades from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowUpgrade {
    pub flow_key: String,
    /// The host's revision of the stored bytes and everything the upgrade depends on.
    pub source_revision: String,
    /// The upgraded document.
    pub document: FlowDocument,
    /// Whether the stored flow was replaced.
    pub applied: bool,
}

/// The application's stored flows.
#[async_trait]
pub trait FlowHost: Send + Sync {
    /// Every stored flow, in key order.
    async fn list(&self) -> Result<Vec<FlowSummary>, Refusal>;
    /// The stored flow `flow_key`. Reading never upgrades it.
    async fn get(&self, flow_key: &str) -> Result<FlowDocument, Refusal>;
    /// The stored flow `flow_key`, validated against the host's catalogue.
    async fn validate(&self, flow_key: &str) -> Result<FlowValidationResult, Refusal>;
    /// The upgrade of `flow_key` the host would apply, without applying it.
    async fn plan_upgrade(&self, flow_key: &str) -> Result<FlowUpgrade, Refusal>;
    /// Applies the upgrade of `flow_key` when its source revision is `expected_revision`.
    async fn apply_upgrade(
        &self,
        flow_key: &str,
        expected_revision: &str,
    ) -> Result<FlowUpgrade, Refusal>;
}

/// A request to activate a node of a stored flow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Activation {
    pub flow_key: String,
    pub node_id: String,
    /// The activation's parameter values, by name.
    #[serde(default)]
    pub parameters: Map<String, Value>,
}

/// What a host reports of one run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunStatus {
    pub run_id: String,
    /// The host's name for the run's state, such as `running`, `succeeded` or `cancelled`.
    pub status: String,
    /// Whether the run has finished and will report no further events.
    pub terminal: bool,
    /// Host detail about the run, such as its outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Value")]
    pub detail: Option<Value>,
}

/// The application's runs.
#[async_trait]
pub trait RunHost: Send + Sync {
    /// Starts `activation` under the caller's grant, sends each execution event to `events`
    /// as it happens, and returns the run's outcome. The host decides the grant the run's tool
    /// calls are made under.
    async fn activate(
        &self,
        auth: &OAuthContext,
        activation: Activation,
        events: &RecordSink,
    ) -> Result<RunStatus, Refusal>;
    /// The run `run_id` as it is now.
    async fn status(&self, run_id: &str) -> Result<RunStatus, Refusal>;
    /// Sends the run's execution events to `events` as they happen, and returns its status
    /// once it is terminal.
    async fn watch(&self, run_id: &str, events: &RecordSink) -> Result<RunStatus, Refusal>;
    /// Requests cancellation of the run `run_id`.
    async fn cancel(&self, run_id: &str) -> Result<RunStatus, Refusal>;
    /// Explicitly resumes the durable execution of the run `run_id`.
    async fn resume(&self, run_id: &str) -> Result<RunStatus, Refusal>;
}

/// The identity of the grant a run's agent calls its tools under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GrantIdentity {
    pub issuer: String,
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub client_id: Option<String>,
}

impl GrantIdentity {
    /// The identity of the verified grant `auth`.
    pub fn of(auth: &OAuthContext) -> Self {
        Self {
            issuer: auth.issuer().into(),
            subject: auth.subject().into(),
            client_id: auth.client_id().map(Into::into),
        }
    }
}

/// The application's durable tool approval journals.
#[async_trait]
pub trait ApprovalHost: Send + Sync {
    /// The durable tool executor of the run `run_id`, opened on its journal.
    async fn approvals(&self, run_id: &str) -> Result<Arc<DurableToolExecutor>, Refusal>;
    /// The identity of the grant the run's agent calls its tools under.
    async fn agent(&self, run_id: &str) -> Result<GrantIdentity, Refusal>;
}

/// One MCP connection the application holds, and the binding its tools arrive through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpServer {
    pub server_id: String,
    /// The revision of the binding new runs take.
    pub binding_revision: String,
    pub credential_scope_revision: String,
}

/// The application's MCP connections.
#[async_trait]
pub trait McpHost: Send + Sync {
    /// Every connection, in server-identity order.
    async fn servers(&self) -> Result<Vec<McpServer>, Refusal>;
    /// Rediscovers the catalogue of `server_id` and rebinds it. The host recomposes the set it
    /// serves; new runs take the new selection, and running conversations keep their pinned
    /// binding.
    async fn refresh(&self, server_id: &str) -> Result<McpServer, Refusal>;
}
