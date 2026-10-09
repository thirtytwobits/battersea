use battersea_pianola::*;
use serde_json::{json, Value};
use std::collections::HashMap;
#[test]
fn pianola_tap_captures_only_matching_ports_in_full_and_in_order() {
    // Port-only target matches that port on any node; node-scoped
    // target matches only its node. Untenanted ports are ignored.
    let mut tap = PianolaTap::new(vec![
        PianolaTapTarget {
            flow_key: "test-flow".to_string(),
            node_id: None,
            port: "user_response".to_string(),
        },
        PianolaTapTarget {
            flow_key: "test-flow".to_string(),
            node_id: Some("parser-1".to_string()),
            port: "out_of_story".to_string(),
        },
    ]);

    // Matches (port-only), full value preserved.
    tap.capture(
        "test-flow",
        "session-output-1",
        "user_response",
        "prompt.fragment",
        &serde_json::json!("hello, world"),
    );
    // Matches (node-scoped to parser-1).
    tap.capture(
        "test-flow",
        "parser-1",
        "out_of_story",
        "prompt.fragment",
        &serde_json::json!({"note": "aside"}),
    );
    // Same port but WRONG node → ignored.
    tap.capture(
        "test-flow",
        "other-node",
        "out_of_story",
        "prompt.fragment",
        &serde_json::json!("ignored"),
    );
    // Untenanted port → ignored.
    tap.capture(
        "test-flow",
        "session-output-1",
        "user_response_stream",
        "prompt.fragment",
        &serde_json::json!("ignored"),
    );

    // A nested/background flow may reuse both the node and port names.
    tap.capture(
        "other-flow",
        "parser-1",
        "out_of_story",
        "prompt.fragment",
        &serde_json::json!("foreign flow"),
    );

    let captured = tap.emissions_from(0);
    assert_eq!(
        captured.len(),
        2,
        "only the two matching emissions are captured"
    );
    // Order + full value + ordinal monotonic.
    assert_eq!(captured[0].port, "user_response");
    assert_eq!(captured[0].value, serde_json::json!("hello, world"));
    assert_eq!(captured[0].ordinal, 0);
    assert_eq!(captured[1].node_id, "parser-1");
    assert_eq!(captured[1].port, "out_of_story");
    assert_eq!(captured[1].value, serde_json::json!({"note": "aside"}));
    assert_eq!(captured[1].ordinal, 1);

    // `emissions_from` is a windowing slice (per-step attribution).
    assert_eq!(tap.emissions_from(1).len(), 1);
    assert_eq!(tap.emissions_from(2).len(), 0);
    assert_eq!(tap.captured_len(), 2);
}
fn raw(passed: bool, detail: Option<&str>) -> RawAssertion {
    RawAssertion {
        summary: "x".to_string(),
        passed,
        detail: detail.map(str::to_string),
    }
}
fn graded_exp(score: f64, max: f64) -> ExpectationReport {
    ExpectationReport {
        summary: "g".to_string(),
        all_criteria_held: score >= max,
        required_criteria_held: true,
        severity: Severity::Graded,
        score: Some(score),
        max: Some(max),
        detail: None,
        pending: false,
        grader: None,
        require_distinct_model: false,
        subject_backend: None,
        subject_model: None,
        rubric: None,
        subject: None,
    }
}
fn step(
    all_criteria_held: bool,
    required_criteria_held: bool,
    expectations: Vec<ExpectationReport>,
) -> StepReport {
    StepReport {
        step_id: "s".to_string(),
        all_criteria_held,
        required_criteria_held,
        error_class: None,
        strike_error: None,
        expectations,
    }
}
#[test]
fn severity_maps_to_criterion_truth_and_grade_contribution() {
    // hard
    let hard_ok = classify_assertion(raw(true, None), Severity::Hard, 1.0);
    assert!(hard_ok.all_criteria_held);
    assert!(hard_ok.required_criteria_held);
    let hard_fail = classify_assertion(raw(false, Some("violated")), Severity::Hard, 1.0);
    assert!(!hard_fail.all_criteria_held);
    assert!(!hard_fail.required_criteria_held);
    assert!(hard_fail.score.is_none());

    // graded: held → full weight; missed → 0 but still not required.
    let graded_ok = classify_assertion(raw(true, None), Severity::Graded, 3.0);
    assert!(graded_ok.all_criteria_held);
    assert!(graded_ok.required_criteria_held);
    assert_eq!(graded_ok.score, Some(3.0));
    assert_eq!(graded_ok.max, Some(3.0));
    let graded_miss = classify_assertion(raw(false, Some("missing data")), Severity::Graded, 3.0);
    assert!(!graded_miss.all_criteria_held);
    assert!(graded_miss.required_criteria_held);
    assert_eq!(graded_miss.score, Some(0.0));
    assert_eq!(graded_miss.detail.as_deref(), Some("missing data"));

    // advisory: missed → recorded, never gates/grades.
    let advisory = classify_assertion(raw(false, None), Severity::Advisory, 1.0);
    assert!(!advisory.all_criteria_held);
    assert!(advisory.required_criteria_held);
    assert!(advisory.score.is_none());
}
#[test]
fn aggregate_grade_is_weighted_ratio_or_none() {
    assert_eq!(aggregate_grade(&[step(true, true, vec![])]), None);
    // 3/3 + 0/1 = 3/4 = 0.75
    let steps = vec![step(
        false,
        true,
        vec![graded_exp(3.0, 3.0), graded_exp(0.0, 1.0)],
    )];
    assert_eq!(aggregate_grade(&steps), Some(0.75));
}
#[test]
fn gate_fails_on_hard_failures_only_by_default() {
    // No hard fail → pass.
    let ok = vec![step(false, true, vec![graded_exp(0.0, 1.0)])];
    assert!(compute_gate(&ok, Some(0.0), None));
    // A hard-fail step gates.
    assert!(!compute_gate(&[step(false, false, vec![])], None, None));
    // An EXCUSED step never gates.
    assert!(compute_gate(&[step(false, true, vec![])], None, None));
    // A hard-fail expectation gates through the step requirement.
    let hard_exp = ExpectationReport {
        summary: "h".to_string(),
        all_criteria_held: false,
        required_criteria_held: false,
        severity: Severity::Hard,
        score: None,
        max: None,
        detail: Some("bad".to_string()),
        pending: false,
        grader: None,
        require_distinct_model: false,
        subject_backend: None,
        subject_model: None,
        rubric: None,
        subject: None,
    };
    assert!(!compute_gate(
        &[step(false, false, vec![hard_exp])],
        None,
        None
    ));
}
#[test]
fn gate_honours_min_grade_threshold() {
    let policy = RollSuccessPolicy {
        require: None,
        min_grade: Some(0.8),
    };
    let steps = vec![step(false, true, vec![graded_exp(0.0, 1.0)])];
    // grade 0.9 ≥ 0.8 → pass; 0.5 < 0.8 → fail.
    assert!(compute_gate(&steps, Some(0.9), Some(&policy)));
    assert!(!compute_gate(&steps, Some(0.5), Some(&policy)));
    // min_grade set but NO graded criteria → fail (asked for a grade that doesn't exist).
    assert!(!compute_gate(
        &[step(true, true, vec![])],
        None,
        Some(&policy)
    ));
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Domain {
    kind: String,
}
impl DomainCriterion for Domain {
    fn kind(&self) -> &str {
        &self.kind
    }
    fn severity(&self) -> Severity {
        Severity::Hard
    }
    fn weight(&self) -> f64 {
        1.0
    }
}
fn extension() -> Value {
    json!({
     "setup": {"type":"object", "additionalProperties":false},
     "actions":[{"type":"object", "additionalProperties":false, "required":["kind","port","value"], "properties":{
       "kind":{"const":"emit"}, "port":{"type":"string"}, "value":{}}}]
    })
}
#[test]
fn independent_roll_captures_asserts_and_resolves_a_grade() {
    let yaml = include_str!("../fixtures/echo.roll.yaml");
    let schema = schema::compose_schema(extension()).unwrap();
    assert!(schema::validate_yaml(yaml, &schema).is_empty());
    let roll: Roll<Value, Value, Expectation<Domain>> = serde_yaml::from_str(yaml).unwrap();
    let step = &roll.steps[0];
    let port = step.action["port"].as_str().unwrap();
    let value = &step.action["value"];
    let mut tap = PianolaTap::new(vec![PianolaTapTarget {
        flow_key: roll.name.clone(),
        node_id: None,
        port: port.into(),
    }]);
    tap.capture(&roll.name, "external-node", port, "text", value);
    let captured = tap.emissions_from(0);
    assert_eq!(captured[0].value, *value);
    let timeline = Timeline {
        ports: captured
            .iter()
            .map(|e| PortEmission {
                port: e.port.clone(),
                value: e.value.clone(),
                at: Some(e.at),
            })
            .collect(),
        ..Timeline::default()
    };
    let registry = AssertionRegistry::<Domain, Timeline>::default();
    let mut reports: Vec<_> = step
        .expect
        .iter()
        .map(|e| registry.evaluate(&step.id, e, &timeline, &HashMap::new(), "output"))
        .collect();
    assert!(reports[..2].iter().all(|e| e.required_criteria_held));
    let grade = &mut reports[2];
    assert!(grade.pending);
    assert_eq!(grade.subject.as_deref(), value.as_str());
    let before = grade.clone();
    assert!(resolve_grade(grade, f64::NAN, "invalid".into(), None).is_err());
    assert_eq!(*grade, before);
    let score = 0.75;
    resolve_grade(grade, score, "scored by host".into(), None).unwrap();
    assert!(!grade.pending);
    assert_eq!(grade.score, grade.max.map(|weight| weight * score));
}
#[test]
fn schema_rejects_unknown_assertions_and_misspelled_matchers() {
    let schema = schema::compose_schema(extension()).unwrap();
    let good = include_str!("../fixtures/echo.roll.yaml");
    assert!(
        !schema::validate_yaml(&good.replace("kind: port", "kind: invented"), &schema).is_empty()
    );
    assert!(!schema::validate_yaml(&good.replace("equals:", "equalz:"), &schema).is_empty());
}
#[test]
fn domain_registration_is_explicit_and_cannot_replace_an_existing_assertion() {
    let mut registry = AssertionRegistry::<Domain, Timeline>::default();
    let e = Expectation::Domain(Domain {
        kind: "owned".into(),
    });
    let timeline = Timeline::default();
    let prior = HashMap::new();
    assert!(
        !registry
            .evaluate("step", &e, &timeline, &prior, "output")
            .required_criteria_held
    );
    registry
        .register("owned", |_, _| pass("host contract".into()))
        .unwrap();
    assert!(registry
        .register("owned", |_, _| fail("replacement".into(), "bad".into()))
        .is_err());
    assert!(
        registry
            .evaluate("step", &e, &timeline, &prior, "output")
            .required_criteria_held
    );
}
#[test]
fn grade_separation_requires_both_identities_and_a_different_model() {
    let mut report = graded_exp(0.0, 2.0);
    report.pending = true;
    report.score = None;
    report.require_distinct_model = true;
    assert!(resolve_grade(&mut report, 1.0, String::new(), Some("grader")).is_err());
    report.subject_model = Some("subject".into());
    assert!(resolve_grade(&mut report, 1.0, String::new(), Some("subject")).is_err());
    resolve_grade(&mut report, 1.0, String::new(), Some("grader")).unwrap();
    assert!(report.all_criteria_held);
}
