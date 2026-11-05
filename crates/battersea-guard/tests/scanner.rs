//! Copyright (c) Scott A Dixon
//! Contract tests for the prompt-source scanner.
use battersea_guard::*;

const PATH: &str = "src/request.rs";

fn policy() -> PromptPolicy {
    PromptPolicy::new(&["prompts/", "config/prompts.toml"], &[r"^src/"]).unwrap()
}

fn scan_with(source: &str, allowlist: &[AllowlistEntry]) -> Vec<(usize, String)> {
    scan_content(&policy(), PATH, source, allowlist)
        .into_iter()
        .map(|violation| (violation.line, violation.rule))
        .collect()
}

fn scan(source: &str) -> Vec<(usize, String)> {
    scan_with(source, &[])
}

fn found(entries: &[(usize, &str)]) -> Vec<(usize, String)> {
    entries
        .iter()
        .map(|(line, rule)| (*line, (*rule).to_string()))
        .collect()
}

#[test]
fn inline_prompt_prose_and_schema_names_are_flagged() {
    assert_eq!(
        scan(r#"let prompt = "You are a careful assistant.";"#),
        found(&[(1, "prompt-prose")])
    );
    assert_eq!(
        scan(r#"let request = Request { schema_name: "plan_request" };"#),
        found(&[(1, "schema-name")])
    );
}

#[test]
fn a_raw_string_literal_after_its_key_is_flagged() {
    for (rule, source) in [
        (
            "schema-name",
            r###"Request { schema_name: r#"plan_request"# }"###,
        ),
        (
            "tool-description",
            r###"Tool { description: r#"Lists files."# }"###,
        ),
        (
            "tool-description",
            r###"json!({"description": r"Lists files."})"###,
        ),
        (
            "system-prompt",
            r###"json!({"system": r#"Answer briefly."#})"###,
        ),
        (
            "planner-fields",
            r###"Plan { summary: Some(r#"A short plan."#) }"###,
        ),
    ] {
        assert_eq!(scan(source), found(&[(1, rule)]), "{source}");
    }
}

#[test]
fn markdown_and_xml_prompt_structure_is_flagged() {
    for source in [
        r###"out.push_str("## Steps");"###,
        r#"out.push_str("intro\n# Title");"#,
        r#"out.push_str("Keep **every** field");"#,
        r#"out.push_str("```json");"#,
    ] {
        assert_eq!(
            scan(source),
            found(&[(1, "markdown-formatter-structure")]),
            "{source}"
        );
    }
    for source in [
        r#"out.push_str("<rules>none</rules>");"#,
        r#"out.push_str("</rules>");"#,
        r#"out.push_str("<break/>");"#,
    ] {
        assert_eq!(
            scan(source),
            found(&[(1, "xml-formatter-structure")]),
            "{source}"
        );
    }
}

#[test]
fn a_description_written_as_a_rust_struct_field_is_flagged() {
    for source in [
        r#"Tool { description: "Lists the workspace files.".into() }"#,
        r#"json!({"description": "Lists the workspace files."})"#,
        // The literal continues onto the next line with a trailing `\`.
        "Tool { description: \"Lists the workspace \\",
    ] {
        assert_eq!(scan(source), found(&[(1, "tool-description")]), "{source}");
    }
}

#[test]
fn a_literal_that_starts_on_the_line_after_its_key_is_flagged() {
    let source = "Tool {\n    description:\n        \"Lists the workspace files.\".into(),\n}";
    let violations = scan_content(&policy(), PATH, source, &[]);
    assert_eq!(violations.len(), 1, "{violations:?}");
    assert_eq!(violations[0].line, 2);
    assert_eq!(violations[0].rule, "tool-description");
    assert_eq!(
        violations[0].excerpt,
        r#"description: "Lists the workspace files.".into(),"#
    );
}

#[test]
fn wrapped_literals_are_flagged_for_every_rule() {
    for (rule, key, literal) in [
        ("prompt-prose", "label", r#""You are a planner.""#),
        ("markdown-formatter-structure", "label", r###""## Steps""###),
        (
            "xml-formatter-structure",
            "label",
            r#""<rules>none</rules>""#,
        ),
        ("schema-name", "schema_name", r#""plan_request""#),
        ("tool-description", "description", r#""Lists files.""#),
        ("system-prompt", r#""system""#, r#""Answer briefly.""#),
        ("planner-fields", "summary", r#""A short plan.""#),
        ("planner-fields", "prompt_text", r##"r#"A lighthouse."#"##),
        ("planner-fields", "negative_prompt", r#""Blurred edges.""#),
    ] {
        let source = format!("    {key}:\n        {literal},");
        let first_line: Vec<_> = scan(&source)
            .into_iter()
            .filter(|(line, _)| *line == 1)
            .collect();
        assert_eq!(first_line, found(&[(1, rule)]), "{source}");
    }
}

#[test]
fn xml_structure_split_across_statements_is_flagged() {
    let source = "out.push_str(\"<rules>\");\nout.push_str(&rules);\nout.push_str(\"</rules>\");";
    assert_eq!(scan(source), found(&[(3, "xml-formatter-structure")]));
}

#[test]
fn a_trailing_colon_that_precedes_code_is_not_flagged() {
    let source = "let description:\n    &str = config.description();\n'summary:\nloop {\n    break 'summary;\n}";
    assert_eq!(scan(source), found(&[]));
}

#[test]
fn a_description_sourced_from_configuration_is_not_flagged() {
    for source in [
        "Tool { description: config.tool_description.clone() }",
        r#"Tool { description: prompts.get("tool.search")? }"#,
        "Tool {\n    description:\n        prompts.get(\"tool.search\")?,\n}",
    ] {
        assert_eq!(scan(source), found(&[]), "{source}");
    }
}

#[test]
fn a_lone_opening_tag_is_not_flagged() {
    assert_eq!(scan(r#"out.push_str("<context>");"#), found(&[]));
}

#[test]
fn generic_type_notation_is_not_flagged() {
    for source in [
        "let index: HashMap<String, Vec<u8>> = HashMap::new();",
        r#"anyhow::bail!("expected HashMap<String, Vec<u8>>");"#,
        r#"let name = "Option<&str>";"#,
    ] {
        assert_eq!(scan(source), found(&[]), "{source}");
    }
}

#[test]
fn only_candidate_paths_are_scanned_and_test_paths_are_exempt() {
    let source = r#"let prompt = "You are a careful assistant.";"#;
    let policy = policy();
    assert!(!scan_content(&policy, PATH, source, &[]).is_empty());
    assert!(scan_content(&policy, "docs/request.rs", source, &[]).is_empty());
    for path in [
        "src/tests/request.rs",
        "src/web/__tests__/request.ts",
        "src/request_test.rs",
        "src/request.test.ts",
        "src/request.spec.mjs",
        "src/request.test.tsx",
    ] {
        assert!(is_test_source_path(path), "{path}");
        assert!(
            scan_content(&policy, path, source, &[]).is_empty(),
            "{path}"
        );
    }
    for path in [
        "src/my_tests/request.rs",
        "src/latest.rs",
        "src/request.test.rs",
    ] {
        assert!(!is_test_source_path(path), "{path}");
        assert!(
            !scan_content(&policy, path, source, &[]).is_empty(),
            "{path}"
        );
    }
}

#[test]
fn rust_test_scopes_and_test_functions_are_skipped_without_swallowing_live_source() {
    let live = r#"const LIVE: &str = "You are live.";"#;
    for scope in [
        "#[cfg(test)]\nmod tests {\n    const P: &str = \"You are a fixture.\";\n}",
        "mod tests {\n    const P: &str = \"You are a fixture.\";\n}",
        "pub(crate) mod tests {\n    const P: &str = \"You are a fixture.\";\n}",
        "#[test]\nfn fixture() {\n    let p = \"You are a fixture.\";\n}",
        "#[tokio::test(flavor = \"current_thread\")]\nasync fn fixture() {\n    let p = \"You are a fixture.\";\n}",
        "#[test]\n#[ignore]\n\nfn fixture()\n{\n    let p = \"You are a fixture.\";\n}",
        "#[cfg(test)]\nconst P: &str = \"You are a fixture.\";",
        "#[cfg(test)]\nfn fixture() {\n    if true {\n        let p = \"You are a fixture.\";\n    }\n}",
    ] {
        let source = format!("{scope}\n{live}");
        let line = source.lines().count();
        assert_eq!(scan(&source), found(&[(line, "prompt-prose")]), "{source}");
    }
}

#[test]
fn strings_and_comments_do_not_move_the_test_source_boundary() {
    // A `}` counted from a string or comment would end the test scope early and
    // expose the fixture that follows it.
    let closes_early = concat!(
        "#[cfg(test)]\n",
        "mod tests {\n",
        "    const RAW: &str = r#\"\n",
        "}\n",
        "}\"#;\n",
        "    const QUOTED: &str = \"}\\\"}\";\n",
        "    // }\n",
        "    const P: &str = \"You are a fixture.\";\n",
        "}\n",
        "const LIVE: &str = \"You are live.\";",
    );
    assert_eq!(scan(closes_early), found(&[(10, "prompt-prose")]));

    // A `{` counted from a string or comment would keep the test scope open and
    // hide the live source after it.
    let stays_open = concat!(
        "#[cfg(test)]\n",
        "mod tests {\n",
        "    const RAW: &str = r##\"\n",
        "{ \"# {\n",
        "\"##;\n",
        "    const QUOTED: &str = \"{\";\n",
        "    // {\n",
        "}\n",
        "const LIVE: &str = \"You are live.\";",
    );
    assert_eq!(scan(stays_open), found(&[(9, "prompt-prose")]));
}

#[test]
fn an_allow_comment_exempts_the_line_after_it() {
    let source = concat!(
        "// ai-prompt-trojan-allow prompt-prose\n",
        "\n",
        "const P: &str = \"You are allowed.\";\n",
        "const Q: &str = \"You are not.\";\n",
        "const R: &str = \"You are allowed.\"; // ai-prompt-trojan-allow prompt-prose",
    );
    assert_eq!(scan(source), found(&[(4, "prompt-prose")]));
}

#[test]
fn an_allow_comment_exempts_the_block_it_opens() {
    let source = concat!(
        "// ai-prompt-trojan-allow prompt-prose\n",
        "fn fixtures() -> [&'static str; 2] {\n",
        "    if true {\n",
        "        let p = \"You are a.\";\n",
        "    }\n",
        "    [\"You are b.\", \"You are c.\"]\n",
        "}\n",
        "const AFTER: &str = \"You are d.\";",
    );
    assert_eq!(scan(source), found(&[(8, "prompt-prose")]));
}

#[test]
fn an_allow_comment_is_scoped_to_its_rule_and_a_wildcard_exempts_every_rule() {
    let line = r#"Tool { description: "You are a search tool." }"#;
    assert_eq!(
        scan(line),
        found(&[(1, "prompt-prose"), (1, "tool-description")])
    );
    let scoped = format!("// ai-prompt-trojan-allow tool-description\n{line}");
    assert_eq!(scan(&scoped), found(&[(2, "prompt-prose")]));
    let wildcard = format!("// ai-prompt-trojan-allow *\n{line}");
    assert_eq!(scan(&wildcard), found(&[]));
    let block = format!(
        "// ai-prompt-trojan-allow tool-description\nfn tools() -> Tool {{\n    {line}\n}}"
    );
    assert_eq!(scan(&block), found(&[(3, "prompt-prose")]));
}

#[test]
fn an_allowlist_entry_matches_only_on_its_exact_triple() {
    let source = r#"let prompt = "You are a careful assistant.";"#;
    let entry = |path: &str, rule: &str, excerpt: &str| AllowlistEntry {
        path: path.into(),
        rule: rule.into(),
        excerpt: excerpt.into(),
        reason: "Reviewed fixture.".into(),
    };
    assert_eq!(
        scan_with(source, &[entry(PATH, "prompt-prose", source)]),
        found(&[])
    );
    for near_miss in [
        entry("src/other.rs", "prompt-prose", source),
        entry(PATH, "tool-description", source),
        entry(PATH, "prompt-prose", "\"You are a careful assistant.\""),
        entry(PATH, "prompt-prose", &format!("{source} ")),
    ] {
        assert_eq!(
            scan_with(source, std::slice::from_ref(&near_miss)),
            found(&[(1, "prompt-prose")]),
            "{near_miss:?}"
        );
    }
}

#[test]
fn a_malformed_allowlist_entry_is_refused() {
    let parsed = parse_allowlist(
        r#"[{"path":"src/a.rs","rule":"prompt-prose","excerpt":"x","reason":"Reviewed."}]"#,
    )
    .unwrap();
    assert_eq!(
        parsed,
        [AllowlistEntry {
            path: "src/a.rs".into(),
            rule: "prompt-prose".into(),
            excerpt: "x".into(),
            reason: "Reviewed.".into(),
        }]
    );
    for raw in [
        r#"[{"path":"src/a.rs","rule":"prompt-prose","excerpt":"x","reason":" \t"}]"#,
        r#"[{"path":"src/a.rs","rule":"prompt-prose","excerpt":"x"}]"#,
        r#"[{"path":"src/a.rs","rule":"prompt-prose","reason":"Reviewed."}]"#,
        r#"{"path":"src/a.rs","rule":"prompt-prose","excerpt":"x","reason":"Reviewed."}"#,
        "not json",
    ] {
        assert!(parse_allowlist(raw).is_err(), "{raw}");
    }
}

#[test]
fn the_report_names_every_violation_and_the_policys_sources_of_truth() {
    let policy = policy();
    let violations = scan_content(
        &policy,
        PATH,
        "let request = Request { schema_name: \"plan_request\" };\nout.push_str(\"</rules>\");",
        &[],
    );
    assert_eq!(violations.len(), 2, "{violations:?}");
    let report = render_report(&policy, &violations);
    assert!(report.contains("prompts/, config/prompts.toml"), "{report}");
    for violation in &violations {
        for part in [
            format!("{}:{}", violation.path, violation.line),
            format!("[{}]", violation.rule),
            violation.message.clone(),
            violation.excerpt.clone(),
        ] {
            assert!(report.contains(&part), "{part:?} missing from {report}");
        }
    }
}
