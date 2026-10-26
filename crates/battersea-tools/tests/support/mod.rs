//! Copyright (c) Scott A Dixon
//! Clapham operations whose handlers are the tests' tool executors.
#![allow(dead_code)]
use battersea_model::*;
use battersea_tools::*;
use clapham::{
    compose_with_options, Bounds, Call, Category, ComposeOptions, ComposedSet, CompositionError,
    Contribution, Effect, Evidence, ExecutionMode, Locality, OAuthContext, Operation,
    OperationName, Outcome, Refusal,
};
use serde_json::{json, Map, Value};
use std::{collections::BTreeMap, sync::Arc};

pub fn object(value: Value) -> Map<String, Value> {
    value.as_object().expect("an object").clone()
}

/// A daemon-local mutation named `name` whose input satisfies `input`.
pub fn operation(name: &str, input: Map<String, Value>) -> Operation {
    Operation {
        name: OperationName::new(name).expect("a valid operation name"),
        title: None,
        description: "Looks a key up.".into(),
        input,
        output: None,
        effect: Effect::Mutation,
        scopes: Vec::new(),
        policies: Vec::new(),
        locality: Locality::Daemon,
        refusals: BTreeMap::new(),
        streams: false,
        preview: false,
        positional: Vec::new(),
        confirmation: false,
        idempotent: false,
        open_world: false,
    }
}

pub fn with_output(mut operation: Operation, output: Value) -> Operation {
    operation.output = Some(serde_json::from_value(output).expect("a JSON Schema"));
    operation
}

pub fn grant() -> OAuthContext {
    OAuthContext::from_verified("https://issuer.test", "battersea", "agent", ["tools"])
}

/// The executor's result as a Clapham outcome, its text blocks as MCP content blocks.
fn outcome(result: EngineLocalToolResult) -> Outcome {
    match result {
        EngineLocalToolResult::Json { content } => Outcome::new(content),
        EngineLocalToolResult::Content(result) => Outcome {
            result: result.structured_content,
            content: Some(
                result
                    .content_blocks
                    .into_iter()
                    .map(|block| match block {
                        ToolContentBlock::Text { text } => json!({"type": "text", "text": text}),
                        block => panic!("unsupported test block {block:?}"),
                    })
                    .collect(),
            ),
            is_error: result.is_error,
            evidence: Evidence::Completed,
        },
    }
}

/// A handler that runs `executor` with the call's own name and validated input.
pub fn handler(
    executor: Arc<dyn EngineLocalToolExecutor>,
) -> impl Fn(
    Call,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Outcome, Refusal>> + Send>>
       + Send
       + Sync {
    move |call: Call| {
        let executor = executor.clone();
        Box::pin(async move {
            executor
                .call(EngineLocalToolCall {
                    id: String::new(),
                    name: call.name.to_string(),
                    arguments: call.input,
                })
                .await
                .map(outcome)
                .map_err(|error| {
                    Refusal::new("executor_failed", Category::Internal, error.message)
                        .with_evidence(Evidence::Uncertain)
                })
        })
    }
}

/// The set composed from `operations`, each run by its executor, under the namespace of the
/// first name.
pub fn compose(
    operations: Vec<(Operation, Arc<dyn EngineLocalToolExecutor>)>,
) -> Result<ComposedSet, CompositionError> {
    let mut contributions: BTreeMap<String, Contribution> = BTreeMap::new();
    for (mut operation, executor) in operations {
        operation
            .refusals
            .insert("executor_failed".into(), Category::Internal);
        let namespace = operation.name.namespace().unwrap_or_default().to_string();
        let contribution = contributions
            .remove(&namespace)
            .unwrap_or_else(|| Contribution::new(namespace.clone()));
        contributions.insert(
            namespace,
            contribution.operation(operation, handler(executor)),
        );
    }
    compose_with_options(
        contributions.into_values(),
        Bounds::default(),
        ComposeOptions {
            execution: ExecutionMode::Daemon,
            ..ComposeOptions::default()
        },
    )
}

/// A registry over every operation of `operations`, under the test grant.
pub fn registry(operations: Vec<(Operation, Arc<dyn EngineLocalToolExecutor>)>) -> ToolRegistry {
    let names = operations
        .iter()
        .map(|(operation, _)| operation.name.to_string())
        .collect::<Vec<_>>();
    let set = compose(operations).expect("a composed set");
    let selection = set.select(&names).expect("composed names");
    ToolRegistry::new(set, selection, grant()).expect("a registry")
}
