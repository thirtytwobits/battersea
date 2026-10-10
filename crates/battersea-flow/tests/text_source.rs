use battersea_flow::{
    document::{load_document, serialize_canonical_document},
    dsl::{export_flow_dsl, parse_flow_dsl},
    FlowDocument, FlowExecutionLimits,
};
use serde_json::json;

fn fixture() -> FlowDocument {
    serde_json::from_value(json!({
        "version": 2, "flow_key": "quotes\".雪", "title": "A\nflow",
        "description": "round trip", "execution": {
            "source_order": ["z", "a"], "limits": FlowExecutionLimits::default()
        },
        "output_encoding":"plain", "plain_fragment_delimiter":"newline", "whitespace_mode":"preserve",
        "nodes": [
            {"id":"z", "definition_name":"Custom.Source", "instance_name":"snow 雪",
             "parameter_values":{"nested":[null,true,42,-2,1.25,{"escaped":"\\\"\n// } ;"}]},
             "port_names":{"input":{"a":"renamed"}}, "port_order":{"input":["b","a"]},
             "port_parameter_values":{"input":{"a":{"value":[1,2]}}}},
            {"id":"a", "definition_name":"Custom.Sink", "instance_name":"A"}
        ],
        "edges":[
            {"id":"e2","kind":"signal","source_node_id":"a","source_port":"after",
             "target_node_id":"z","target_port":"disable","order":7},
            {"id":"e1","kind":"token","source_node_id":"z","source_port":"out",
             "target_node_id":"a","target_port":"@gain","order":3,
             "queue":{"items":2,"bytes":128,"max_event_bytes":64,"policy":"drop_oldest"}}
        ],
        "layout":{"canvas":{"positions":{"z":{"x":-10.25,"y":18}},"zoom":0.75},"other":[1,"x"]},
        "metadata":{"opaque":{"n":null,"s":"text"}}
    })).unwrap()
}

#[test]
fn all_document_fields_and_authored_orders_survive_text_and_json() {
    let expected = fixture();
    let text = export_flow_dsl(&expected).unwrap();
    let parsed = parse_flow_dsl(&text).unwrap();
    assert_eq!(parsed, expected);
    assert_eq!(export_flow_dsl(&parsed).unwrap(), text);
    let json = serialize_canonical_document(&expected).unwrap();
    let from_json = load_document(&json).unwrap();
    assert_eq!(
        parse_flow_dsl(&export_flow_dsl(&from_json).unwrap()).unwrap(),
        from_json
    );
    assert_eq!(serialize_canonical_document(&parsed).unwrap(), json);
}

#[test]
fn authored_source_handles_comments_multiline_json_and_quoted_identifiers() {
    let limits = serde_json::to_string(&FlowExecutionLimits::default()).unwrap();
    let source = format!(
        r#"// authored by a person
flow 1 "manual" {{
 version = 2;
 title = "Manual";
 execution = {{"source_order":["s"],"limits":{limits}}};
 node "s" "Source" {{"instance_name":"S", "parameter_values":{{"text":"https://x"}}}}
 node "t" "Sink" {{"instance_name":"T"}}
 // Ordered token and signal routing
 edge "token" "s"."out" -> "t"."@gain" {{"order": 2}}
 edge "signal" "s"."after" ~> "t"."disable" {{}}
 layout = {{"editor":{{"positions":{{"s":[-5,10]}}}}}};
}}"#
    );
    let document = parse_flow_dsl(&source).unwrap();
    let expected: FlowDocument = serde_json::from_value(json!({
        "version":2,"flow_key":"manual","title":"Manual",
        "execution":{"source_order":["s"],"limits":FlowExecutionLimits::default()},
        "nodes":[{"id":"s","definition_name":"Source","instance_name":"S","parameter_values":{"text":"https://x"}},
                 {"id":"t","definition_name":"Sink","instance_name":"T"}],
        "edges":[{"id":"token","kind":"token","source_node_id":"s","source_port":"out","target_node_id":"t","target_port":"@gain","order":2},
                 {"id":"signal","kind":"signal","source_node_id":"s","source_port":"after","target_node_id":"t","target_port":"disable"}],
        "layout":{"editor":{"positions":{"s":[-5,10]}}}
    })).unwrap();
    assert_eq!(document, expected);
}

#[test]
fn ambiguous_or_unsupported_sources_fail_with_locations() {
    let valid = export_flow_dsl(&fixture()).unwrap();
    for broken in [
        valid.replacen("flow 1", "flow 999", 1),
        valid.replacen("version = 2", "version = 1", 1),
        valid.replacen("title =", "typo =", 1),
        valid.replacen("title =", "version = 2; title =", 1),
        valid.replacen("node \"z\"", "node \"a\"", 1),
        valid.replacen("edge \"e2\"", "edge \"e1\"", 1),
        valid.replacen(
            "\"instance_name\":",
            "\"id\":\"shadow\",\"instance_name\":",
            1,
        ),
        valid.replacen(
            "\"instance_name\":",
            "\"instance_name\":\"duplicate\",\"instance_name\":",
            1,
        ),
        valid.replacen("\"order\":", "\"kind\":\"signal\",\"order\":", 1),
        valid.replacen(" -> ", " => ", 1),
        valid.replacen("version = 2;", "version = 2", 1),
        format!("{valid} garbage"),
        valid[..valid.len() / 2].to_string(),
    ] {
        let error = parse_flow_dsl(&broken).expect_err(&broken);
        assert!(error.line > 0 && error.column > 0);
        assert!(!error.message.is_empty());
    }
    let error = parse_flow_dsl("flow 1 \"x\" {\n ?\n}").unwrap_err();
    assert_eq!((error.line, error.column), (2, 2));
}

#[test]
fn arbitrary_json_strings_and_truncated_sources_never_lose_data_or_panic() {
    for value in [
        "",
        "\0",
        "\r\n",
        "雪🦀",
        "\\\"{[;]}//",
        "a.b ~> c",
        "\u{2028}",
    ] {
        let mut flow = fixture();
        flow.flow_key = value.into();
        flow.nodes[0]
            .parameter_values
            .insert("string".into(), json!(value));
        let text = export_flow_dsl(&flow).unwrap();
        assert_eq!(parse_flow_dsl(&text).unwrap(), flow);
        for (end, _) in text.char_indices() {
            let _ = parse_flow_dsl(&text[..end]);
        }
    }
}

#[test]
fn exporter_rejects_versions_and_duplicate_identities() {
    let mut flow = fixture();
    flow.version += 1;
    assert!(export_flow_dsl(&flow).is_err());
    let mut flow = fixture();
    flow.nodes.push(flow.nodes[0].clone());
    assert!(export_flow_dsl(&flow).is_err());
    let mut flow = fixture();
    flow.edges.push(flow.edges[0].clone());
    assert!(export_flow_dsl(&flow).is_err());
}

#[test]
fn typed_errors_point_to_the_authored_declaration() {
    let valid = export_flow_dsl(&fixture()).unwrap();
    let broken = valid.replacen("\"instance_name\":", "\"typo\":", 1);
    let line = broken
        .lines()
        .position(|line| line.contains("typo"))
        .unwrap()
        + 1;
    assert_eq!(parse_flow_dsl(&broken).unwrap_err().line, line);
    let broken = valid.replacen("version = 2", "version = 99", 1);
    let line = broken
        .lines()
        .position(|line| line.contains("version ="))
        .unwrap()
        + 1;
    assert_eq!(parse_flow_dsl(&broken).unwrap_err().line, line);
}
