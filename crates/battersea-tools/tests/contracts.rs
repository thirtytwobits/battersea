mod support;

use async_trait::async_trait;
use battersea_model::*;
use battersea_tools::*;
use clapham::{CompositionCode, Effect, OperationName};
use serde_json::{json, Map, Value};
use std::sync::{Arc, Mutex};
use support::{object, operation, with_output};

/// An executor that records each call it runs and answers with its arguments.
#[derive(Default)]
struct Recorder(Mutex<Vec<EngineLocalToolCall>>);
#[async_trait]
impl EngineLocalToolExecutor for Recorder {
    async fn call(
        &self,
        call: EngineLocalToolCall,
    ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
        let content = call.arguments.clone();
        self.0.lock().unwrap().push(call);
        Ok(EngineLocalToolResult::json(content))
    }
}
impl Recorder {
    fn calls(&self) -> Vec<EngineLocalToolCall> {
        self.0.lock().unwrap().clone()
    }
}

fn keyed_schema() -> Map<String, Value> {
    object(
        json!({"type":"object","properties":{"key":{"type":"string"}},"required":["key"],"additionalProperties":false}),
    )
}

fn lookup(name: &str) -> clapham::Operation {
    operation(name, keyed_schema())
}

fn recorded(
    recorder: &Arc<Recorder>,
    operations: impl IntoIterator<Item = clapham::Operation>,
) -> ToolRegistry {
    support::registry(
        operations
            .into_iter()
            .map(|operation| {
                (
                    operation,
                    recorder.clone() as Arc<dyn EngineLocalToolExecutor>,
                )
            })
            .collect(),
    )
}

fn call(name: &str, arguments: Value) -> EngineLocalToolCall {
    EngineLocalToolCall {
        id: "call".into(),
        name: name.into(),
        arguments,
    }
}

#[tokio::test]
async fn a_call_is_checked_against_its_operation_before_its_handler_runs() {
    let recorder = Arc::new(Recorder::default());
    let registry = recorded(&recorder, [lookup("example.lookup")]);
    for (name, arguments, code) in [
        (
            "example.lookup",
            json!({"key": false}),
            ToolCode::InvalidToolArguments,
        ),
        (
            "example.unregistered",
            json!({"key": "selected"}),
            ToolCode::UnregisteredTool,
        ),
    ] {
        let error = registry.call(call(name, arguments)).await.unwrap_err();
        assert_eq!(error.tool_code, Some(code), "{name}");
    }
    assert!(recorder.calls().is_empty());
}

#[test]
fn operations_whose_provider_names_collide_are_refused() {
    let recorder = Arc::new(Recorder::default());
    let set = clapham::compose(
        [
            clapham::Contribution::new("a")
                .operation(lookup("a.b_c"), support::handler(recorder.clone())),
            clapham::Contribution::new("a_b")
                .operation(lookup("a_b.c"), support::handler(recorder)),
        ],
        clapham::Bounds::default(),
    )
    .unwrap();
    let error = ToolRegistry::new(
        set.clone(),
        set.select(["a.b_c", "a_b.c"]).unwrap(),
        support::grant(),
    )
    .err()
    .unwrap();
    assert_eq!(error.tool_code, Some(ToolCode::DuplicateTool));
    assert!(ToolRegistry::new(
        set.clone(),
        set.select(["a.b_c"]).unwrap(),
        support::grant()
    )
    .is_ok());
}

#[tokio::test]
async fn a_handler_receives_a_call_under_its_operations_own_name() {
    let recorder = Arc::new(Recorder::default());
    let registry = recorded(&recorder, [lookup("example.lookup")]);
    let offered = registry.definitions();
    assert_eq!(offered.len(), 1);
    let arguments = json!({"key": "selected"});
    for name in [offered[0].name.as_str(), "example.lookup"] {
        let result = registry.call(call(name, arguments.clone())).await.unwrap();
        assert_eq!(result, EngineLocalToolResult::json(arguments.clone()));
    }
    let calls = recorder.calls();
    assert_eq!(calls.len(), 2);
    assert!(calls
        .iter()
        .all(|received| received.name == "example.lookup"));
}

#[test]
fn a_provider_is_offered_each_operation_under_its_portable_name_with_its_description() {
    let mut described = lookup("example.lookup");
    described.description = "  Looks a key up.\n\n  Indented detail.\n".into();
    let registry = recorded(&Arc::default(), [described.clone()]);
    let offered = registry.definitions();
    assert_eq!(offered.len(), 1);
    assert_eq!(offered[0].name, local_tool_name("example.lookup"));
    assert_eq!(offered[0].description, described.description);
    assert_eq!(offered[0].input_schema, Value::Object(keyed_schema()));
    assert_eq!(
        registry.tools().collect::<Vec<_>>(),
        vec![&Tool::for_operation(&described)]
    );
}

#[test]
fn a_descriptor_that_cannot_be_offered_is_refused_by_clapham() {
    for name in ["", "example lookup", &format!("e.{}", "n".repeat(127))] {
        assert!(OperationName::new(name).is_err(), "{name:?}");
    }
    assert!(OperationName::new(format!("e.{}", "n".repeat(126))).is_ok());
    let mut blank = lookup("example.lookup");
    blank.description = " ".into();
    let refused = [
        (blank, CompositionCode::MissingDescription),
        (
            operation("example.lookup", object(json!({"type": "array"}))),
            CompositionCode::InvalidSchema,
        ),
        (
            operation("example.lookup", Map::new()),
            CompositionCode::InvalidSchema,
        ),
        (
            operation(
                "example.lookup",
                object(json!({"type": "object", "minProperties": "one"})),
            ),
            CompositionCode::InvalidSchema,
        ),
        (
            with_output(
                lookup("example.lookup"),
                json!({"type": "object", "minProperties": "one"}),
            ),
            CompositionCode::InvalidSchema,
        ),
    ];
    for (operation, code) in refused {
        let error = support::compose(vec![(operation, Arc::new(Recorder::default()))])
            .err()
            .unwrap();
        assert_eq!(error.code, code);
    }
    let error = support::compose(
        [lookup("example.lookup"), lookup("example.lookup")]
            .into_iter()
            .map(|operation| {
                (
                    operation,
                    Arc::new(Recorder::default()) as Arc<dyn EngineLocalToolExecutor>,
                )
            })
            .collect(),
    )
    .err()
    .unwrap();
    assert_eq!(error.code, CompositionCode::DuplicateOperation);
}

#[test]
fn profiles_must_be_distinct_non_blank_and_name_selected_operations() {
    let registry = recorded(&Arc::default(), [lookup("example.lookup")]);
    for (profile, names) in [
        (" ", vec!["example.lookup"]),
        ("author", vec!["example.missing"]),
    ] {
        let error = registry.clone().with_profile(profile, names).err().unwrap();
        assert_eq!(error.tool_code, Some(ToolCode::InvalidToolDefinition));
    }
    let error = registry
        .with_profile("author", ["example.lookup"])
        .unwrap()
        .with_profile("author", ["example.lookup"])
        .err()
        .unwrap();
    assert_eq!(error.tool_code, Some(ToolCode::InvalidToolDefinition));
}

#[tokio::test]
async fn a_result_outside_the_declared_output_schema_is_refused() {
    let recorder = Arc::new(Recorder::default());
    let echo = with_output(lookup("example.echo"), Value::Object(keyed_schema()));
    let answers = with_output(
        lookup("example.answers"),
        json!({"type": "object", "required": ["value"], "properties": {"value": {"type": "string"}}}),
    );
    let registry = recorded(&recorder, [echo, answers]);
    let arguments = json!({"key": "selected"});
    let result = registry
        .call(call("example.echo", arguments.clone()))
        .await
        .unwrap();
    assert_eq!(result, EngineLocalToolResult::json(arguments.clone()));
    let error = registry
        .call(call("example.answers", arguments.clone()))
        .await
        .unwrap_err();
    assert_eq!(error.tool_code, Some(ToolCode::InvalidToolResult));
    assert_eq!(error.tool_call.unwrap().outcome, ToolCallOutcome::Completed);
    assert_eq!(
        recorder.calls().len(),
        2,
        "the handler ran before its result was checked"
    );
}

struct FixedResult(EngineLocalToolResult);
#[async_trait]
impl EngineLocalToolExecutor for FixedResult {
    async fn call(
        &self,
        _: EngineLocalToolCall,
    ) -> Result<EngineLocalToolResult, EngineAdapterRequestError> {
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn content_results_validate_structured_output_and_preserve_tool_errors() {
    for (structured_content, is_error, accepted) in [
        (Some(json!({"key": "value"})), false, true),
        (Some(json!({"key": 42})), false, false),
        (None, false, false),
        (Some(Value::Null), false, false),
        (None, true, true),
        (Some(json!({"error": "missing"})), true, true),
    ] {
        let result = EngineLocalToolResult::Content(ToolContentResult {
            content_blocks: vec![ToolContentBlock::Text {
                text: "human-readable result".into(),
            }],
            structured_content,
            is_error,
            meta: Map::new(),
        });
        let registry = support::registry(vec![(
            with_output(lookup("example.lookup"), Value::Object(keyed_schema())),
            Arc::new(FixedResult(result.clone())),
        )]);
        let returned = registry
            .call(call("example.lookup", json!({"key": "query"})))
            .await;
        if accepted {
            assert_eq!(returned.unwrap(), result);
        } else {
            let error = returned.unwrap_err();
            assert_eq!(error.tool_code, Some(ToolCode::InvalidToolResult));
            assert_eq!(error.tool_call.unwrap().outcome, ToolCallOutcome::Completed);
        }
    }
}

#[tokio::test]
async fn registry_refusals_retain_not_dispatched_evidence() {
    let recorder = Arc::new(Recorder::default());
    let registry = recorded(&recorder, [lookup("example.lookup")]);
    for (name, arguments) in [
        ("missing", json!({})),
        ("example.lookup", json!({"key": 3})),
    ] {
        let error = registry.call(call(name, arguments)).await.unwrap_err();
        let evidence = error.tool_call.unwrap();
        assert_eq!(evidence.call_id, "call");
        assert_eq!(evidence.outcome, ToolCallOutcome::NotDispatched);
    }
    assert!(recorder.calls().is_empty());
}

#[tokio::test]
async fn a_refusal_reaches_the_tool_loop_with_its_code_and_evidence() {
    let mut scoped = lookup("example.scoped");
    scoped.scopes = vec!["example:write".into()];
    let recorder = Arc::new(Recorder::default());
    let registry = recorded(&recorder, [scoped]);
    let error = registry
        .call(call("example.scoped", json!({"key": "selected"})))
        .await
        .unwrap_err();
    assert_eq!(error.classification.as_str(), "insufficient_scope");
    assert_eq!(error.tool_code, None);
    assert_eq!(
        error.tool_call.unwrap().outcome,
        ToolCallOutcome::NotDispatched
    );
    assert!(recorder.calls().is_empty());
}

#[tokio::test]
async fn a_profile_is_offered_and_runs_only_the_operations_it_is_permitted() {
    let recorder = Arc::new(Recorder::default());
    let registry = recorded(
        &recorder,
        [
            lookup("example.shared"),
            lookup("example.authoring"),
            lookup("example.unlisted"),
        ],
    )
    .with_profile("author", ["example.shared", "example.authoring"])
    .unwrap()
    .with_profile("player", ["example.shared"])
    .unwrap();
    let player = registry.for_profile("player");
    let shared = player.tools().collect::<Vec<_>>();
    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0].name, "example.shared");
    assert_eq!(shared[0].profiles(), Some(vec!["author", "player"]));
    assert_eq!(
        player
            .definitions()
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>(),
        vec![local_tool_name("example.shared")]
    );
    let error = player
        .call(call("example.authoring", json!({"key": "selected"})))
        .await
        .unwrap_err();
    assert_eq!(error.tool_code, Some(ToolCode::UnregisteredTool));
    assert!(recorder.calls().is_empty());
    let author = registry.for_profile("author");
    let names = author
        .tools()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["example.authoring", "example.shared"]);
    assert!(author.tools().all(|tool| tool.permits("author")));
    assert!(registry
        .tools()
        .find(|tool| tool.name == "example.unlisted")
        .is_some_and(|tool| tool.profiles() == Some(Vec::new())));
    assert_ne!(author.revision(), registry.revision());
    assert_eq!(author.revision(), registry.for_profile("author").revision());
}

#[test]
fn the_presented_record_states_every_hint_from_the_effect() {
    for (effect, idempotent, read_only, destructive, idempotent_hint) in [
        (Effect::Query, false, true, false, true),
        (Effect::Mutation, true, false, false, true),
        (Effect::Mutation, false, false, false, false),
        (Effect::Administrative, false, false, false, false),
        (Effect::Destructive, false, false, true, false),
    ] {
        let mut descriptor = lookup("example.lookup");
        descriptor.effect = effect;
        descriptor.idempotent = idempotent;
        descriptor.open_world = true;
        let tool = Tool::for_operation(&descriptor);
        let annotations = tool.annotations.clone().unwrap();
        assert_eq!(annotations.read_only_hint, Some(read_only));
        assert_eq!(annotations.destructive_hint, Some(destructive));
        assert_eq!(annotations.idempotent_hint, Some(idempotent_hint));
        assert_eq!(annotations.open_world_hint, Some(true));
        assert_eq!(
            tool.meta
                .get("io.github.thirtytwobits.clapham/effect")
                .cloned(),
            (effect == Effect::Administrative).then(|| json!("administrative"))
        );
    }
    let mut streaming = lookup("example.watch");
    streaming.streams = true;
    assert_eq!(
        Tool::for_operation(&streaming).meta["io.github.thirtytwobits.clapham/records"],
        json!(true)
    );
}

#[test]
fn profiles_read_back_as_they_were_listed() {
    let profiles = ["author", "player"];
    let tool = Tool::new("example.lookup", keyed_schema()).with_profiles(profiles);
    assert_eq!(tool.profiles(), Some(profiles.to_vec()));
    assert_eq!(
        Tool::new("example.lookup", keyed_schema()).profiles(),
        Some(Vec::new())
    );
}

#[test]
fn an_unannotated_tool_takes_the_protocols_cautious_defaults() {
    let effect = Tool::new("example.lookup", keyed_schema()).effect();
    assert!(!effect.read_only);
    assert!(effect.destructive);
    assert!(!effect.idempotent);
    assert!(effect.open_world);
    let read_only = Tool::new("example.lookup", keyed_schema())
        .with_annotations(ToolAnnotations::read_only_closed_world())
        .effect();
    assert!(read_only.read_only && !read_only.destructive && read_only.idempotent);
    assert!(!read_only.open_world);
}

#[test]
fn a_tool_record_round_trips_in_the_protocols_spelling() {
    let record = json!({
        "name": "example.lookup",
        "title": "Look a key up",
        "description": "Looks a key up.",
        "inputSchema": keyed_schema(),
        "outputSchema": {"type": "object"},
        "annotations": {"title": "Lookup", "readOnlyHint": true, "openWorldHint": false},
        "icons": [{"src": "https://example.test/icon.png", "mimeType": "image/png", "sizes": ["48x48"], "theme": "dark"}],
        "_meta": {PROFILES_META_KEY: ["author"], "com.example/note": {"any": "json"}}
    });
    let tool: Tool = serde_json::from_value(record.clone()).unwrap();
    assert_eq!(tool.profiles(), Some(vec!["author"]));
    assert_eq!(serde_json::to_value(&tool).unwrap(), record);
    let minimal = json!({"name": "example.lookup", "inputSchema": keyed_schema()});
    let tool: Tool = serde_json::from_value(minimal.clone()).unwrap();
    assert_eq!(serde_json::to_value(&tool).unwrap(), minimal);
    assert!(serde_json::from_value::<Tool>(json!({
        "name": "example.lookup", "inputSchema": keyed_schema(), "inputschema": {}
    }))
    .is_err());
}

#[test]
fn the_published_schema_admits_every_record_the_crate_writes() {
    let schema = schema::contract_schema();
    let definitions = schema["$defs"].as_object().unwrap();
    for name in ["Tool", "ToolAnnotations", "Icon", "IconTheme"] {
        assert!(definitions.contains_key(name), "{name}");
    }
    let mut tool_schema = schema.clone();
    tool_schema["$ref"] = json!("#/$defs/Tool");
    tool_schema.as_object_mut().unwrap().remove("properties");
    tool_schema.as_object_mut().unwrap().remove("required");
    let validator = jsonschema::validator_for(&tool_schema).unwrap();
    let mut presented = lookup("example.lookup");
    presented.title = Some("Look a key up".into());
    presented.effect = Effect::Administrative;
    let presented =
        Tool::for_operation(&with_output(presented, json!(true))).with_profiles(["author"]);
    for tool in [Tool::new("example.lookup", keyed_schema()), presented] {
        let record = serde_json::to_value(&tool).unwrap();
        assert!(validator.is_valid(&record), "{record}");
    }
    assert!(!validator.is_valid(&json!({"name": "example.lookup"})));
}

#[test]
fn a_record_requires_the_same_fields_to_be_read_as_to_be_written() {
    use schemars::generate::SchemaSettings;
    let required = |settings: SchemaSettings| {
        let mut generator = settings.into_generator();
        generator.subschema_for::<Tool>();
        ["Tool", "ToolAnnotations", "Icon"]
            .map(|name| generator.definitions()[name].get("required").cloned())
    };
    assert_eq!(
        required(SchemaSettings::draft2020_12().for_serialize()),
        required(SchemaSettings::draft2020_12().for_deserialize())
    );
}
