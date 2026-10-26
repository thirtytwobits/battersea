//! Copyright (c) Scott A Dixon
//! The `battersea` contribution: one operation per host capability the application supplies.
use crate::{codes, host::*};
use battersea_flow::{FlowDocument, FlowValidationResult};
use battersea_model::{EngineAdapterRequestError, EngineLocalToolResult, ToolCode};
use battersea_tools::{
    StoredToolCallState, ToolApprovalAction, ToolApprovalJournal, ToolApprovalReconciliation,
};
use clapham::{
    Call, Contribution, Effect, Evidence, Locality, OAuthContext, Refusal, TypedOperation,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{future::Future, sync::Arc};

/// The namespace Battersea's operations are contributed under.
pub const NAMESPACE: &str = "battersea";

/// The key of a stored flow.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FlowKey {
    pub flow_key: String,
}

/// Every stored flow.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Flows {
    pub flows: Vec<FlowSummary>,
}

/// An upgrade of a stored flow, fenced on the source revision a preview reported.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpgradeRequest {
    pub flow_key: String,
    /// Required to apply the upgrade; a preview reports the revision to supply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub expected_revision: Option<String>,
}

/// The identity of a run.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunId {
    pub run_id: String,
}

/// A host decision on one pending call.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAction {
    Allow,
    Deny,
    Cancel,
}

/// A decision on one call of a run's approval journal.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub run_id: String,
    pub call_id: String,
    /// The call's request revision, as `approval.list` reports it.
    pub request_revision: String,
    /// The journal revision the decision was made at.
    pub expected_revision: u64,
    pub action: DecisionAction,
}

/// How an unresolved dispatch intent ended, as the host established at the effect's destination.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "outcome", deny_unknown_fields)]
pub enum Resolution {
    /// The effect was not applied; the call returns to approved.
    NotApplied,
    /// The effect completed with `result`, a tool result.
    Completed {
        #[schemars(with = "Value")]
        result: Value,
    },
    /// The call ends without a result.
    Abandon,
}

/// The reconciliation of one unresolved call of a run's approval journal.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Reconciliation {
    pub run_id: String,
    pub call_id: String,
    pub expected_revision: u64,
    pub resolution: Resolution,
    /// The evidence the host relied on.
    pub proof: String,
}

/// One call of a run's approval journal.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApprovalCall {
    pub call_id: String,
    pub tool_name: String,
    #[schemars(with = "Value")]
    pub arguments: Value,
    pub request_revision: String,
    /// `pending`, `approved`, `denied`, `cancelled`, `intent`, `completed`, `failed` or
    /// `abandoned`.
    pub state: String,
}

/// A run's approval journal at one revision.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Approvals {
    pub run_id: String,
    pub revision: u64,
    pub calls: Vec<ApprovalCall>,
}

/// No input.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Nothing {}

/// Every MCP connection.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpServers {
    pub servers: Vec<McpServer>,
}

/// The server identity of an MCP connection.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McpServerId {
    pub server_id: String,
}

/// Battersea's operations over the host capabilities the application supplies. Each capability
/// contributes its operations; a capability left out contributes none.
#[derive(Clone, Default)]
pub struct BatterseaOps {
    flows: Option<Arc<dyn FlowHost>>,
    runs: Option<Arc<dyn RunHost>>,
    approvals: Option<Arc<dyn ApprovalHost>>,
    mcp: Option<Arc<dyn McpHost>>,
}

fn descriptor<I: JsonSchema, O: JsonSchema>(
    local: &str,
    description: &str,
    effect: Effect,
    scope: &str,
) -> TypedOperation<I, O> {
    let mut operation = TypedOperation::new(format!("{NAMESPACE}.{local}"), description, effect)
        .expect("Battersea's operation names are valid")
        .locality(Locality::Daemon)
        .require_scope(scope);
    for (code, category) in codes::declared() {
        operation = operation.refusal(code, category);
    }
    operation
}

fn tool_refusal(error: EngineAdapterRequestError) -> Refusal {
    let code = error
        .tool_code
        .map_or(codes::HOST_FAILURE, ToolCode::as_str);
    let refusal = codes::refusal(code, error.message.to_string());
    if error.tool_code == Some(ToolCode::ToolApprovalStorage) {
        refusal.with_evidence(Evidence::Uncertain)
    } else {
        refusal
    }
}

fn approvals(run_id: String, journal: ToolApprovalJournal) -> Approvals {
    let calls = journal
        .calls
        .into_iter()
        .map(|(call_id, call)| ApprovalCall {
            call_id,
            tool_name: call.tool.name,
            arguments: call.call.arguments,
            request_revision: call.request_revision,
            state: match call.state {
                StoredToolCallState::Pending => "pending",
                StoredToolCallState::Approved => "approved",
                StoredToolCallState::Denied => "denied",
                StoredToolCallState::Cancelled => "cancelled",
                StoredToolCallState::Intent => "intent",
                StoredToolCallState::Completed { .. } => "completed",
                StoredToolCallState::Failed { .. } => "failed",
                StoredToolCallState::Abandoned => "abandoned",
            }
            .into(),
        })
        .collect();
    Approvals {
        run_id,
        revision: journal.revision,
        calls,
    }
}

/// The decision's actor: the grant's issuer and subject, with its client where present.
fn actor(auth: &OAuthContext) -> String {
    serde_json::to_string(&GrantIdentity::of(auth)).expect("a serialisable identity")
}

impl BatterseaOps {
    pub fn new() -> Self {
        Self::default()
    }

    /// Contributes `battersea.flow.list`, `.get`, `.validate` and `.upgrade` over `flows`.
    pub fn with_flows(mut self, flows: Arc<dyn FlowHost>) -> Self {
        self.flows = Some(flows);
        self
    }

    /// Contributes `battersea.run.activate`, `.status`, `.watch`, `.cancel` and `.resume` over
    /// `runs`.
    pub fn with_runs(mut self, runs: Arc<dyn RunHost>) -> Self {
        self.runs = Some(runs);
        self
    }

    /// Contributes `battersea.approval.list`, `.decide` and `.reconcile` over `approvals`.
    pub fn with_approvals(mut self, approvals: Arc<dyn ApprovalHost>) -> Self {
        self.approvals = Some(approvals);
        self
    }

    /// Contributes `battersea.mcp.servers` and `.refresh` over `mcp`.
    pub fn with_mcp(mut self, mcp: Arc<dyn McpHost>) -> Self {
        self.mcp = Some(mcp);
        self
    }

    /// The `battersea` contribution.
    pub fn contribution(&self) -> Contribution {
        let mut contribution = Contribution::new(NAMESPACE);
        if let Some(flows) = &self.flows {
            contribution = flow_operations(contribution, flows.clone());
        }
        if let Some(runs) = &self.runs {
            contribution = run_operations(contribution, runs.clone());
        }
        if let Some(approvals) = &self.approvals {
            contribution = approval_operations(contribution, approvals.clone());
        }
        if let Some(mcp) = &self.mcp {
            contribution = mcp_operations(contribution, mcp.clone());
        }
        contribution
    }
}

/// A typed handler that calls `run` with a clone of the host.
fn over<H, I, O, F, Fut>(
    host: Arc<H>,
    run: F,
) -> impl Fn(Call, I) -> std::pin::Pin<Box<dyn Future<Output = Result<O, Refusal>> + Send>>
       + Send
       + Sync
       + 'static
where
    H: ?Sized + Send + Sync + 'static,
    I: Send + 'static,
    O: 'static,
    F: Fn(Arc<H>, Call, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<O, Refusal>> + Send + 'static,
{
    move |call, input| Box::pin(run(host.clone(), call, input))
}

fn flow_operations(contribution: Contribution, flows: Arc<dyn FlowHost>) -> Contribution {
    let read = "battersea:flows:read";
    contribution
        .typed_operation(
            descriptor::<Nothing, Flows>(
                "flow.list",
                "List the stored flows.",
                Effect::Query,
                read,
            ),
            over(flows.clone(), |flows, _, _: Nothing| async move {
                Ok(Flows {
                    flows: flows.list().await?,
                })
            }),
        )
        .typed_operation(
            descriptor::<FlowKey, FlowDocument>(
                "flow.get",
                "Read a stored flow document without upgrading it.",
                Effect::Query,
                read,
            )
            .positional(["flow_key"]),
            over(flows.clone(), |flows, _, input: FlowKey| async move {
                flows.get(&input.flow_key).await
            }),
        )
        .typed_operation(
            descriptor::<FlowKey, FlowValidationResult>(
                "flow.validate",
                "Validate a stored flow against the catalogue and report its issues.",
                Effect::Query,
                read,
            )
            .positional(["flow_key"]),
            over(flows.clone(), |flows, _, input: FlowKey| async move {
                flows.validate(&input.flow_key).await
            }),
        )
        .typed_operation(
            descriptor::<UpgradeRequest, FlowUpgrade>(
                "flow.upgrade",
                "Upgrade a stored flow's document format, fenced on the source revision a \
                 preview reports.",
                Effect::Administrative,
                "battersea:flows:admin",
            )
            .positional(["flow_key"])
            .preview(true),
            over(
                flows,
                |flows, call: Call, input: UpgradeRequest| async move {
                    if call.preview {
                        let plan = flows.plan_upgrade(&input.flow_key).await?;
                        if input
                            .expected_revision
                            .as_ref()
                            .is_some_and(|expected| *expected != plan.source_revision)
                        {
                            return Err(codes::refusal(
                                codes::STALE_UPGRADE_REVISION,
                                "The flow or its catalogue changed; preview the upgrade again.",
                            ));
                        }
                        return Ok(plan);
                    }
                    let expected = input.expected_revision.ok_or_else(|| {
                        codes::refusal(
                            codes::UPGRADE_REVISION_REQUIRED,
                            "Preview the upgrade and supply its source revision.",
                        )
                        .with_evidence(Evidence::NotDispatched)
                    })?;
                    flows.apply_upgrade(&input.flow_key, &expected).await
                },
            ),
        )
}

fn run_operations(contribution: Contribution, runs: Arc<dyn RunHost>) -> Contribution {
    let (read, write) = ("battersea:runs:read", "battersea:runs:write");
    contribution
        .typed_operation(
            descriptor::<Activation, RunStatus>(
                "run.activate",
                "Activate a node of a stored flow, streaming its execution events, then its \
                 outcome.",
                Effect::Mutation,
                write,
            )
            .positional(["flow_key", "node_id"])
            .streams(true)
            .open_world(true),
            over(
                runs.clone(),
                |runs, call: Call, input: Activation| async move {
                    runs.activate(&call.auth, input, &call.records).await
                },
            ),
        )
        .typed_operation(
            descriptor::<RunId, RunStatus>(
                "run.status",
                "Report a run's state.",
                Effect::Query,
                read,
            )
            .positional(["run_id"]),
            over(runs.clone(), |runs, _, input: RunId| async move {
                runs.status(&input.run_id).await
            }),
        )
        .typed_operation(
            descriptor::<RunId, RunStatus>(
                "run.watch",
                "Stream a run's execution events as they happen, then its terminal state.",
                Effect::Query,
                read,
            )
            .positional(["run_id"])
            .streams(true),
            over(runs.clone(), |runs, call: Call, input: RunId| async move {
                runs.watch(&input.run_id, &call.records).await
            }),
        )
        .typed_operation(
            descriptor::<RunId, RunStatus>(
                "run.cancel",
                "Request cancellation of a run.",
                Effect::Mutation,
                write,
            )
            .positional(["run_id"])
            .idempotent(true),
            over(runs.clone(), |runs, _, input: RunId| async move {
                runs.cancel(&input.run_id).await
            }),
        )
        .typed_operation(
            descriptor::<RunId, RunStatus>(
                "run.resume",
                "Explicitly resume a run's durable execution.",
                Effect::Administrative,
                "battersea:runs:admin",
            )
            .positional(["run_id"])
            .open_world(true),
            over(runs, |runs, _, input: RunId| async move {
                runs.resume(&input.run_id).await
            }),
        )
}

fn approval_operations(contribution: Contribution, hosts: Arc<dyn ApprovalHost>) -> Contribution {
    contribution
        .typed_operation(
            descriptor::<RunId, Approvals>(
                "approval.list",
                "List the calls of a run's tool approval journal and their states.",
                Effect::Query,
                "battersea:approvals:read",
            )
            .positional(["run_id"]),
            over(hosts.clone(), |hosts, _, input: RunId| async move {
                let journal = hosts
                    .approvals(&input.run_id)
                    .await?
                    .inspect()
                    .await
                    .map_err(tool_refusal)?;
                Ok(approvals(input.run_id, journal))
            }),
        )
        .typed_operation(
            descriptor::<Decision, Approvals>(
                "approval.decide",
                "Allow, deny or cancel a pending tool call by its request revision.",
                Effect::Mutation,
                "battersea:approvals:decide",
            )
            .positional(["run_id", "call_id"])
            .idempotent(true),
            over(
                hosts.clone(),
                |hosts, call: Call, input: Decision| async move {
                    if hosts.agent(&input.run_id).await? == GrantIdentity::of(&call.auth) {
                        return Err(codes::refusal(
                            codes::OWN_GRANT_DECISION,
                            "The run's own agent grant cannot decide its tool calls.",
                        )
                        .with_evidence(Evidence::NotDispatched));
                    }
                    let action = match input.action {
                        DecisionAction::Allow => ToolApprovalAction::Allow,
                        DecisionAction::Deny => ToolApprovalAction::Deny,
                        DecisionAction::Cancel => ToolApprovalAction::Cancel,
                    };
                    let journal = hosts
                        .approvals(&input.run_id)
                        .await?
                        .decide(
                            &input.call_id,
                            &input.request_revision,
                            input.expected_revision,
                            action,
                            &actor(&call.auth),
                        )
                        .await
                        .map_err(tool_refusal)?;
                    Ok(approvals(input.run_id, journal))
                },
            ),
        )
        .typed_operation(
            descriptor::<Reconciliation, Approvals>(
                "approval.reconcile",
                "Resolve a call's unresolved dispatch intent as not applied, completed with a \
                 result, or abandoned, with the evidence relied on.",
                Effect::Administrative,
                "battersea:approvals:admin",
            )
            .positional(["run_id", "call_id"]),
            over(
                hosts,
                |hosts, call: Call, input: Reconciliation| async move {
                    let resolution = match input.resolution {
                        Resolution::NotApplied => ToolApprovalReconciliation::NotApplied,
                        Resolution::Abandon => ToolApprovalReconciliation::Abandon,
                        Resolution::Completed { result } => ToolApprovalReconciliation::Completed {
                            result: serde_json::from_value::<EngineLocalToolResult>(result)
                                .map_err(|error| {
                                    codes::refusal(codes::INVALID_RECONCILIATION, error.to_string())
                                        .with_evidence(Evidence::NotDispatched)
                                })?,
                        },
                    };
                    let journal = hosts
                        .approvals(&input.run_id)
                        .await?
                        .reconcile(
                            &input.call_id,
                            input.expected_revision,
                            resolution,
                            &actor(&call.auth),
                            &input.proof,
                        )
                        .await
                        .map_err(tool_refusal)?;
                    Ok(approvals(input.run_id, journal))
                },
            ),
        )
}

fn mcp_operations(contribution: Contribution, mcp: Arc<dyn McpHost>) -> Contribution {
    contribution
        .typed_operation(
            descriptor::<Nothing, McpServers>(
                "mcp.servers",
                "List the MCP connections and the binding revision new runs take.",
                Effect::Query,
                "battersea:mcp:read",
            ),
            over(mcp.clone(), |mcp, _, _: Nothing| async move {
                Ok(McpServers {
                    servers: mcp.servers().await?,
                })
            }),
        )
        .typed_operation(
            descriptor::<McpServerId, McpServer>(
                "mcp.refresh",
                "Rediscover an MCP connection's catalogue and rebind it for new runs.",
                Effect::Administrative,
                "battersea:mcp:admin",
            )
            .positional(["server_id"]),
            over(mcp, |mcp, _, input: McpServerId| async move {
                mcp.refresh(&input.server_id).await
            }),
        )
}
