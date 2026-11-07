//! Copyright (c) Scott A Dixon
//!
//! The port: what an application supplies for the kit's handlers, in the kit's terms.
use crate::{
    compaction::{Policy, PolicyEvaluation},
    CallRecord, JobId, JobState, Memory, Reservation, Revision, RunState, SpendEstimate, Summary,
    ToolPlan, TranscriptId, Turn, TurnId,
};
use async_trait::async_trait;
use battersea_model::{EngineAdapter, EngineTokenUsage};
use battersea_runtime::ExecutionHost;
use serde::{de::DeserializeOwned, Serialize};
use std::sync::Arc;

/// The application's transcript, memory, model, tools, spend, compaction and output, reached
/// by the kit's handlers. `contracts/kit-conversation.md` states each operation's contract.
#[async_trait]
pub trait Port: ExecutionHost {
    /// What the application adds to the kit's records.
    type Extension: Serialize + DeserializeOwned + Clone + Send + Sync + 'static;
    /// The kit's state in the run.
    fn run_state(state: &mut Self::State) -> &mut RunState<Self::Extension>;

    /// The transcript's committed turns, in order.
    async fn turns(
        &self,
        state: &Self::State,
        transcript: &TranscriptId,
    ) -> Result<Vec<Turn<Self::Extension>>, Self::Error>;
    /// Stages `turn` on the run's transcript for the application to commit, assigning its
    /// `id` and `created_at`.
    async fn stage(
        &self,
        state: &mut Self::State,
        turn: Turn<Self::Extension>,
    ) -> Result<TurnId, Self::Error>;
    /// Replaces a committed turn's content and counts the revision.
    async fn revise(
        &self,
        state: &mut Self::State,
        revision: Revision<Self::Extension>,
    ) -> Result<(), Self::Error>;

    /// The transcript's latest summary and the turns after it.
    async fn memory(
        &self,
        state: &Self::State,
        transcript: &TranscriptId,
    ) -> Result<Memory<Self::Extension>, Self::Error>;
    /// Writes `summary` as the run's transcript's memory.
    async fn write_summary(
        &self,
        state: &mut Self::State,
        summary: Summary<Self::Extension>,
    ) -> Result<(), Self::Error>;

    /// The model `node` calls.
    fn engine(
        &self,
        state: &Self::State,
        node: &str,
    ) -> Result<Arc<dyn EngineAdapter>, Self::Error>;
    /// The tools `node` offers the model.
    fn tool_plan(&self, state: &Self::State, node: &str) -> Result<ToolPlan, Self::Error>;
    /// Sets spend aside for a call; a refusal is `conversation.spend_refused`.
    async fn reserve(
        &self,
        state: &mut Self::State,
        node: &str,
        estimate: SpendEstimate,
    ) -> Result<Reservation, Self::Error>;
    /// Settles a reservation with what the call used.
    async fn settle(
        &self,
        state: &mut Self::State,
        reservation: Reservation,
        usage: EngineTokenUsage,
    ) -> Result<(), Self::Error>;
    async fn record_call(
        &self,
        state: &mut Self::State,
        record: CallRecord,
    ) -> Result<(), Self::Error>;

    /// The compaction policy in force for `transcript`, the flow's overrides applied.
    fn policy(&self, state: &Self::State, transcript: &TranscriptId)
        -> Result<Policy, Self::Error>;
    /// Requests a compaction job for the evaluation's transcript.
    async fn request_compaction(
        &self,
        state: &mut Self::State,
        evaluation: PolicyEvaluation,
    ) -> Result<JobId, Self::Error>;
    /// Moves a job to `to`; a job the port no longer knows is `conversation.unknown_job`.
    async fn transition(
        &self,
        state: &mut Self::State,
        job: &JobId,
        to: JobState,
    ) -> Result<(), Self::Error>;

    /// Delivers `text` to the application as the run's output.
    async fn deliver(&self, state: &mut Self::State, text: String) -> Result<(), Self::Error>;
}
