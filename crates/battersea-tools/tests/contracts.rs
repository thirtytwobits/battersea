use async_trait::async_trait;
use battersea_model::*;
use battersea_tools::*;
use serde_json::{json, Map, Value};
use std::sync::{Arc, Mutex};

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
        Ok(EngineLocalToolResult { content })
    }
}
impl Recorder {
    fn calls(&self) -> Vec<EngineLocalToolCall> {
        self.0.lock().unwrap().clone()
    }
}

fn object(value: Value) -> Map<String, Value> {
    value.as_object().expect("an object").clone()
}

fn keyed_schema() -> Map<String, Value> {
    object(
        json!({"type":"object","properties":{"key":{"type":"string"}},"required":["key"],"additionalProperties":false}),
    )
}

fn lookup(name: &str) -> Tool {
    Tool::new(name, keyed_schema()).with_description("Looks a key up.")
}

fn call(name: &str, arguments: Value) -> EngineLocalToolCall {
    EngineLocalToolCall {
        id: "call".into(),
        name: name.into(),
        arguments,
    }
}

#[tokio::test]
async fn a_call_is_checked_against_its_tool_before_its_executor_runs() {
    let recorder = Arc::new(Recorder::default());
    let mut registry = ToolRegistry::default();
    registry
        .register(lookup("example.lookup"), recorder.clone())
        .unwrap();
    let collision = registry
        .register(lookup("example_lookup"), recorder.clone())
        .unwrap_err();
    assert_eq!(collision.tool_code, Some(ToolCode::DuplicateTool));
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

#[tokio::test]
async fn an_executor_receives_a_call_under_its_tools_own_name() {
    let recorder = Arc::new(Recorder::default());
    let mut registry = ToolRegistry::default();
    let tool = lookup("example.lookup");
    registry.register(tool.clone(), recorder.clone()).unwrap();
    let offered = registry.definitions();
    assert_eq!(offered.len(), 1);
    let arguments = json!({"key": "selected"});
    for name in [offered[0].name.as_str(), tool.name.as_str()] {
        let result = registry.call(call(name, arguments.clone())).await.unwrap();
        assert_eq!(result.content, arguments);
    }
    let calls = recorder.calls();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|received| received.name == tool.name));
}

#[test]
fn a_provider_is_offered_each_tool_under_its_portable_name_with_its_description() {
    let tool =
        lookup("example.lookup").with_description("  Looks a key up.\n\n  Indented detail.\n");
    let mut registry = ToolRegistry::default();
    registry
        .register(tool.clone(), Arc::new(Recorder::default()))
        .unwrap();
    let offered = registry.definitions();
    assert_eq!(offered.len(), 1);
    assert_eq!(offered[0].name, local_tool_name(&tool.name));
    assert_eq!(
        Some(offered[0].description.as_str()),
        tool.description.as_deref()
    );
    assert_eq!(
        offered[0].input_schema,
        Value::Object(tool.input_schema.clone())
    );
    assert_eq!(registry.tools().collect::<Vec<_>>(), vec![&tool]);
}

#[test]
fn a_record_that_cannot_be_offered_is_refused() {
    let mut profiles_not_names = lookup("example.lookup");
    profiles_not_names
        .meta
        .insert(PROFILES_META_KEY.into(), json!("author"));
    let mut not_a_list = lookup("example.lookup");
    not_a_list.meta.insert(PROFILES_META_KEY.into(), json!([1]));
    let refused = [
        ("blank name", lookup("")),
        ("name with a space", lookup("example lookup")),
        ("name too long", lookup(&"n".repeat(129))),
        (
            "no description",
            Tool::new("example.lookup", keyed_schema()),
        ),
        (
            "blank description",
            Tool::new("example.lookup", keyed_schema()).with_description(" "),
        ),
        (
            "array input schema",
            Tool::new("example.lookup", object(json!({"type": "array"})))
                .with_description("Lists."),
        ),
        ("profiles not a list", profiles_not_names),
        ("profiles not names", not_a_list),
        (
            "blank profile",
            lookup("example.lookup").with_profiles(["author", " "]),
        ),
        (
            "repeated profile",
            lookup("example.lookup").with_profiles(["author", "author"]),
        ),
    ];
    for (case, tool) in refused {
        let error = ToolRegistry::default()
            .register(tool, Arc::new(Recorder::default()))
            .unwrap_err();
        assert_eq!(
            error.tool_code,
            Some(ToolCode::InvalidToolDefinition),
            "{case}"
        );
    }
    let mut uncompilable_output = lookup("example.lookup");
    uncompilable_output.output_schema =
        Some(object(json!({"type": "object", "minProperties": "one"})));
    let error = ToolRegistry::default()
        .register(uncompilable_output, Arc::new(Recorder::default()))
        .unwrap_err();
    assert_eq!(error.tool_code, Some(ToolCode::InvalidToolSchema));
    let uncompilable = Tool::new(
        "example.lookup",
        object(json!({"type": "object", "minProperties": "one"})),
    )
    .with_description("Looks a key up.");
    let error = ToolRegistry::default()
        .register(uncompilable, Arc::new(Recorder::default()))
        .unwrap_err();
    assert_eq!(error.tool_code, Some(ToolCode::InvalidToolSchema));
    let longest = lookup(&"n".repeat(128));
    assert!(ToolRegistry::default()
        .register(longest, Arc::new(Recorder::default()))
        .is_ok());
}

#[tokio::test]
async fn a_result_outside_the_declared_output_schema_is_refused() {
    let recorder = Arc::new(Recorder::default());
    let mut registry = ToolRegistry::default();
    let mut echo = lookup("example.echo");
    echo.output_schema = Some(keyed_schema());
    let mut answers = lookup("example.answers");
    answers.output_schema = Some(object(
        json!({"type": "object", "required": ["value"], "properties": {"value": {"type": "string"}}}),
    ));
    for tool in [&echo, &answers] {
        registry.register(tool.clone(), recorder.clone()).unwrap();
    }
    let arguments = json!({"key": "selected"});
    let result = registry
        .call(call(&echo.name, arguments.clone()))
        .await
        .unwrap();
    assert_eq!(result.content, arguments);
    let error = registry
        .call(call(&answers.name, arguments.clone()))
        .await
        .unwrap_err();
    assert_eq!(error.tool_code, Some(ToolCode::InvalidToolResult));
    assert_eq!(
        recorder.calls().len(),
        2,
        "the executor ran before its result was checked"
    );
}

#[tokio::test]
async fn a_profile_is_offered_and_runs_only_the_tools_it_is_permitted() {
    let recorder = Arc::new(Recorder::default());
    let mut registry = ToolRegistry::default();
    let shared = lookup("example.shared").with_profiles(["author", "player"]);
    let authoring = lookup("example.authoring").with_profiles(["author"]);
    let unlisted = lookup("example.unlisted");
    for tool in [&shared, &authoring, &unlisted] {
        registry.register(tool.clone(), recorder.clone()).unwrap();
    }
    let player = registry.for_profile("player");
    assert_eq!(player.tools().collect::<Vec<_>>(), vec![&shared]);
    assert_eq!(
        player
            .definitions()
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>(),
        vec![local_tool_name(&shared.name)]
    );
    let error = player
        .call(call(&authoring.name, json!({"key": "selected"})))
        .await
        .unwrap_err();
    assert_eq!(error.tool_code, Some(ToolCode::UnregisteredTool));
    assert!(recorder.calls().is_empty());
    let author = registry.for_profile("author");
    let mut names = author
        .tools()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    names.sort_unstable();
    assert_eq!(names, vec![authoring.name.as_str(), shared.name.as_str()]);
    assert!(registry
        .for_profile("author")
        .tools()
        .all(|tool| tool.permits("author")));
    assert!(!unlisted.permits("author"));
}

#[test]
fn profiles_read_back_as_they_were_listed() {
    let profiles = ["author", "player"];
    let tool = lookup("example.lookup").with_profiles(profiles);
    assert_eq!(tool.profiles(), Some(profiles.to_vec()));
    assert_eq!(lookup("example.lookup").profiles(), Some(Vec::new()));
}

#[test]
fn an_unannotated_tool_takes_the_protocols_cautious_defaults() {
    let effect = lookup("example.lookup").effect();
    assert!(!effect.read_only);
    assert!(effect.destructive);
    assert!(!effect.idempotent);
    assert!(effect.open_world);
    let read_only = lookup("example.lookup")
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
    let full = lookup("example.lookup")
        .with_title("Look a key up")
        .with_annotations(ToolAnnotations::read_only_closed_world())
        .with_profiles(["author"]);
    for tool in [Tool::new("example.lookup", keyed_schema()), full] {
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
