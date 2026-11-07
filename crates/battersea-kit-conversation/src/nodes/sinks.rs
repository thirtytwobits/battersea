//! Copyright (c) Scott A Dixon
//!
//! The sinks: output, staging, the compaction request, the memory write and the revision.
//! Each hands what it receives to the port.
use super::{count, messages, provider_failed, record, text_of, turns_of, unsigned};
use crate::{
    compaction::{evaluate, Decision, Policy, Trigger},
    JobState, Port, Revision, Summary,
};
use async_trait::async_trait;
use battersea_flow::{catalog::node_definition, FlowNode};
use battersea_runtime::{ExecutionError, NodeHandler, Retained, Token};

/// `conversation.output`: delivers text to the application.
pub struct OutputHandler;
/// `conversation.transcript.output`: stages each turn it receives, in delivery order.
pub struct TranscriptOutputHandler;
/// `conversation.compaction.request`: evaluates the policy and requests a job on `compact`.
pub struct CompactionRequestHandler;
/// `conversation.memory.output`: writes the summary and completes the job it names.
pub struct MemoryOutputHandler;
/// `conversation.refine.output`: applies a revision to a committed turn.
pub struct RefinementOutputHandler;

#[async_trait]
impl<H: Port> NodeHandler<H> for OutputHandler {
    fn handler_id(&self) -> &'static str {
        "conversation.output"
    }
    async fn execute_sink(
        &self,
        host: &H,
        runtime: &mut H::State,
        _node: &FlowNode,
        _port: &str,
        value: Retained<Token>,
    ) -> Result<(), H::Error> {
        let text = text_of::<H::Error>(&value)?;
        host.deliver(runtime, text).await
    }
}

#[async_trait]
impl<H: Port> NodeHandler<H> for TranscriptOutputHandler {
    fn handler_id(&self) -> &'static str {
        "conversation.transcript.output"
    }
    async fn execute_sink(
        &self,
        host: &H,
        runtime: &mut H::State,
        _node: &FlowNode,
        _port: &str,
        value: Retained<Token>,
    ) -> Result<(), H::Error> {
        for turn in turns_of::<H::Extension, H::Error>(&value)? {
            host.stage(runtime, turn).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl<H: Port> NodeHandler<H> for CompactionRequestHandler {
    fn handler_id(&self) -> &'static str {
        "conversation.compaction.request"
    }
    async fn execute_sink(
        &self,
        host: &H,
        runtime: &mut H::State,
        node: &FlowNode,
        _port: &str,
        value: Retained<Token>,
    ) -> Result<(), H::Error> {
        let definition = node_definition(&runtime.definitions, node)
            .ok_or_else(|| H::Error::internal("The compaction request lost its definition."))?
            .clone();
        let added = turns_of::<H::Extension, H::Error>(&value)?;
        let transcript = H::run_state(runtime).transcript.clone();
        let overrides = Policy {
            max_turns: unsigned(node, &definition, "max_turns"),
            max_tokens: unsigned(node, &definition, "max_tokens"),
            idle: None,
        };
        let policy = host
            .policy(runtime, &transcript)?
            .with_overrides(&overrides);
        let mut memory = host.memory(runtime, &transcript).await?;
        memory.turns.extend(added);
        let engine = host.engine(runtime, &node.id)?;
        let tokens = count(engine.as_ref(), messages(&memory.turns))
            .await
            .map_err(|error| provider_failed::<H::Error>(&error))?;
        let evaluation = evaluate(&policy, Trigger::AfterRun, &memory, tokens);
        match evaluation.decision {
            Decision::Compact => {
                host.request_compaction(runtime, evaluation).await?;
                host.emit_flow_signal(runtime, node, "requested").await
            }
            Decision::Hold => host.emit_flow_signal(runtime, node, "held").await,
        }
    }
}

#[async_trait]
impl<H: Port> NodeHandler<H> for MemoryOutputHandler {
    fn handler_id(&self) -> &'static str {
        "conversation.memory.output"
    }
    async fn execute_sink(
        &self,
        host: &H,
        runtime: &mut H::State,
        _node: &FlowNode,
        _port: &str,
        value: Retained<Token>,
    ) -> Result<(), H::Error> {
        let summary: Summary<H::Extension> = record(&value)?;
        let job = summary.job.clone();
        host.write_summary(runtime, summary).await?;
        match job {
            Some(job) => host.transition(runtime, &job, JobState::Completed).await,
            None => Ok(()),
        }
    }
}

#[async_trait]
impl<H: Port> NodeHandler<H> for RefinementOutputHandler {
    fn handler_id(&self) -> &'static str {
        "conversation.refine.output"
    }
    async fn execute_sink(
        &self,
        host: &H,
        runtime: &mut H::State,
        _node: &FlowNode,
        _port: &str,
        value: Retained<Token>,
    ) -> Result<(), H::Error> {
        let revision: Revision<H::Extension> = record(&value)?;
        host.revise(runtime, revision).await
    }
}
