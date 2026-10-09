//! Copyright (c) Scott A Dixon
//! Prompt-source policy scanner.
use std::collections::BTreeSet;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use regex_lite::Regex;
use serde::Deserialize;

/// One flagged line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub path: String,
    pub line: usize,
    pub rule: String,
    pub message: String,
    pub excerpt: String,
}

/// A reviewed exemption. `reason` must be non-empty: an allowlist entry without
/// a written justification is indistinguishable from a silenced defect.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct AllowlistEntry {
    pub path: String,
    pub rule: String,
    pub excerpt: String,
    pub reason: String,
}

/// Parses and validates the allowlist file's contents.
pub fn parse_allowlist(raw_json: &str) -> Result<Vec<AllowlistEntry>> {
    let entries: Vec<AllowlistEntry> = serde_json::from_str(raw_json)
        .context("prompt trojan allowlist must be a JSON array of {path, rule, excerpt, reason}")?;
    if let Some(index) = entries
        .iter()
        .position(|entry| entry.reason.trim().is_empty())
    {
        anyhow::bail!("prompt trojan allowlist entry {index} must include a non-empty reason",);
    }
    Ok(entries)
}

struct Patterns {
    test_sources: Vec<Regex>,
    rules: Vec<(&'static str, &'static str, Vec<Regex>)>,
    allow_comment: Regex,
    test_attribute: Regex,
    test_module: Regex,
    comment_line: Regex,
    blank_line: Regex,
    attribute_line: Regex,
}

fn compile(pattern: &str) -> Regex {
    Regex::new(pattern)
        .unwrap_or_else(|error| panic!("invalid scanner pattern {pattern:?}: {error}"))
}

fn patterns() -> &'static Patterns {
    static PATTERNS: OnceLock<Patterns> = OnceLock::new();
    PATTERNS.get_or_init(|| Patterns {
        test_sources: [
            r"(^|/)tests/",
            r"(^|/)__tests__/",
            r"\.(test|spec)\.[cm]?[jt]sx?$",
            r"_test\.rs$",
        ]
        .iter()
        .map(|pattern| compile(pattern))
        .collect(),
        rules: vec![
            (
                "prompt-prose",
                "AI-facing prompt prose must come from approved prompt config, not source.",
                vec![compile(
                    r#""(You are|Return only|Write plain text only|Create a structured prompt package|Generate a character image|Selected characters:|Selected [^"]+:|Image-generation rules:|Character identity locks:|Do not include markdown code fences|Do not output JSON|Do not include text outside)"#,
                )],
            ),
            (
                "markdown-formatter-structure",
                "Markdown prompt structure must come from declared prompt roots, not Rust source.",
                vec![
                    compile(r#"(r#*)?"[^"\n]*(^|[^A-Za-z0-9_])#{1,6}[ \t]+(\{|[A-Za-z0-9_]|[A-Z])"#),
                    compile(r#"(r#*)?"[^"\n]*(\\n|\n)#{1,6}[ \t]+(\{|[A-Za-z0-9_]|[A-Z])"#),
                    compile(r#"(r#*)?"[^"\n]*(\*\*[^"\n]+\*\*|```)"#),
                ],
            ),
            (
                "xml-formatter-structure",
                "XML-like prompt structure must come from declared prompt roots, not Rust source.",
                vec![
                    compile(
                        r#"(r#*)?"[^"\n]*(</[A-Za-z{][A-Za-z0-9_:{}/ .="'-]*>|<[A-Za-z{][A-Za-z0-9_:{}/ .="'-]*/>|<[A-Za-z{][A-Za-z0-9_:{}/ .="'-]*>[^"\n]*</[A-Za-z{][A-Za-z0-9_:{}/ .="'-]*>)[^"\n]*""#,
                    ),
                ],
            ),
            (
                "schema-name",
                "Schema names sent to providers must come from approved prompt config.",
                vec![compile(r#"\bschema_name[ \t]*:[ \t]*"[^"]+""#)],
            ),
            (
                "tool-description",
                "Tool and schema-field descriptions sent over the wire must come from approved prompt config.",
                vec![
                    // A JSON-shaped key, as in a hand-built serde_json schema.
                    compile(r#""description"[ \t]*:[ \t]*"[^"]+""#),
                    // A Rust struct field. The rule matched only the quoted form
                    // for a long time, which is how 22 model-facing tool
                    // descriptions sat in chat_tool_catalog.rs unreported: they
                    // were struct fields, so the scan saw nothing.
                    // The closing quote is deliberately not required. Long
                    // descriptions are written with a trailing `\` continuation,
                    // so the literal opens on one line and closes on another;
                    // requiring both on one line is what let the catalogue's 22
                    // descriptions through even after this shape was covered.
                    compile(r#"\bdescription[ \t]*:[ \t]*"[^"]*"#),
                ],
            ),
            (
                "system-prompt",
                "System prompt text must come from approved prompt config.",
                vec![compile(r#""system"[ \t]*:[ \t]*"[^"]+""#)],
            ),
            (
                "planner-fields",
                "Planner prompt text, summary, and negative_prompt literals must come from approved prompt config.",
                vec![compile(
                    r#"\b(summary|prompt_text|negative_prompt)[ \t]*:[ \t]*(Some\()?"[^"]+""#,
                )],
            ),
        ],
        allow_comment: compile(r"ai-prompt-trojan-allow[ \t]+(\*|[a-z0-9-]+)"),
        test_attribute: compile(
            r"^[ \t]*#[ \t]*\[[ \t]*(cfg[ \t]*\([ \t]*test[ \t]*\)|([A-Za-z0-9_:]+::)?test([ \t]*\([^)]*\))?)[ \t]*\]",
        ),
        test_module: compile(r"^[ \t]*(pub(\([^)]*\))?[ \t]+)?mod[ \t]+tests[ \t]*\{"),
        comment_line: compile(r"^[ \t]*//"),
        blank_line: compile(r"^[ \t]*$"),
        attribute_line: compile(r"^[ \t]*#[ \t]*\["),
    })
}

/// Test sources, where prompt-shaped fixtures are expected.
pub fn is_test_source_path(file_path: &str) -> bool {
    patterns()
        .test_sources
        .iter()
        .any(|pattern| pattern.is_match(file_path))
}

/// Which rule a line's allow-comment names, if any. `*` allows every rule.
fn line_allow_rule(line: &str) -> Option<String> {
    patterns()
        .allow_comment
        .captures(line)
        .map(|captures| captures[1].to_string())
}

fn allows_rule(line: &str, rule_id: &str) -> bool {
    matches!(line_allow_rule(line), Some(rule) if rule == "*" || rule == rule_id)
}

fn rules_contain(allow_rules: &BTreeSet<String>, rule_id: &str) -> bool {
    allow_rules
        .iter()
        .any(|rule| rule == "*" || rule == rule_id)
}

/// Line-by-line brace tracking that survives multi-line raw strings.
///
/// Brace depth is how the scanner finds where a `#[cfg(test)]` block ends. A
/// line-local count cannot see a string spanning several lines, so a raw-string
/// fixture like `r#"{ ... }"#` inside a test module reads as a real brace pair:
/// the `{` is hidden inside the string but the closing `}"#` is counted, the
/// block appears to end early, and the rest of the test module gets scanned as
/// live source. The same miscount in the other direction is the dangerous one —
/// it would hide live source inside an apparent test block.
#[derive(Default)]
struct RustBraceTracker {
    /// Number of `#` on the raw string currently open, if any.
    raw_hashes: Option<usize>,
}

impl RustBraceTracker {
    fn delta(&mut self, line: &str) -> i32 {
        let chars: Vec<char> = line.chars().collect();
        let mut delta = 0;
        let mut in_string = false;
        let mut escaped = false;
        let mut index = 0;

        while index < chars.len() {
            let character = chars[index];

            if let Some(hashes) = self.raw_hashes {
                if character == '"'
                    && (0..hashes).all(|offset| chars.get(index + 1 + offset) == Some(&'#'))
                {
                    self.raw_hashes = None;
                    index += 1 + hashes;
                    continue;
                }
                index += 1;
                continue;
            }

            if in_string {
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == '"' {
                    in_string = false;
                }
                index += 1;
                continue;
            }

            if character == '/' && chars.get(index + 1) == Some(&'/') {
                break;
            }

            // `r"..."` / `r#"..."#` — no escapes inside, and it may run past the
            // end of this line.
            if character == 'r' {
                let hashes = (0..)
                    .take_while(|offset| chars.get(index + 1 + offset) == Some(&'#'))
                    .count();
                if chars.get(index + 1 + hashes) == Some(&'"') {
                    self.raw_hashes = Some(hashes);
                    index += 2 + hashes;
                    continue;
                }
            }

            match character {
                '"' => in_string = true,
                '{' => delta += 1,
                '}' => delta -= 1,
                _ => {}
            }
            index += 1;
        }

        delta
    }
}

/// Returns `line` with `next` appended when `line` is a key awaiting a string
/// literal that starts on the following line.
///
/// The scanner matches one line at a time, so `description:` with its literal
/// wrapped onto the next line was invisible to every rule — two of `world.rs`'s
/// field descriptions sat unreported for exactly that reason. rustfmt produces
/// this shape on its own whenever the literal is long enough, so it is the
/// normal way a *long* piece of prompt text looks, not an unusual one.
///
/// The join is deliberately narrow: the line must end at the colon, and the
/// next must open a string literal. A trailing colon otherwise — a loop label,
/// a type ascription broken across lines — joins nothing.
fn join_wrapped_literal(line: &str, next: Option<&str>) -> String {
    let trimmed = line.trim_end();
    let Some(next) = next else {
        return line.to_string();
    };
    if !trimmed.ends_with(':') {
        return line.to_string();
    }
    let continuation = next.trim_start();
    let opens_literal = continuation.starts_with('"')
        || continuation.starts_with("r\"")
        || continuation.starts_with("r#");
    if !opens_literal {
        return line.to_string();
    }
    format!("{trimmed} {continuation}")
}

/// Scans one file's contents.
///
/// Returns nothing for non-candidate, allowed, or test paths, so callers may
/// pass any path without pre-filtering.
pub fn scan_content(
    policy: &PromptPolicy,
    file_path: &str,
    content: &str,
    allowlist: &[AllowlistEntry],
) -> Vec<Violation> {
    if policy.is_prompt_source(file_path)
        || is_test_source_path(file_path)
        || !policy.is_candidate(file_path)
    {
        return Vec::new();
    }

    let patterns = patterns();
    let lines: Vec<&str> = content
        .split('\n')
        .map(|line| line.trim_end_matches('\r'))
        .collect();
    let mut violations = Vec::new();

    let mut pending_test_item = false;
    let mut test_block_depth: Option<i32> = None;
    let mut pending_allow_rules: Vec<String> = Vec::new();
    let mut active_allow_rules: BTreeSet<String> = BTreeSet::new();
    let mut active_allow_depth: Option<i32> = None;

    // One tracker for the whole file, advanced exactly once per line before any
    // branch can `continue` past it — otherwise a line skipped by an early
    // branch would leave its raw-string state unread.
    let mut braces = RustBraceTracker::default();

    for (index, line) in lines.iter().enumerate() {
        let brace_delta = braces.delta(line);

        // Inside a test block: consume until its braces close.
        if let Some(depth) = test_block_depth {
            let next = depth + brace_delta;
            test_block_depth = if next <= 0 { None } else { Some(next) };
            continue;
        }

        // Just saw a test attribute; skip the item it applies to.
        if pending_test_item {
            if patterns.blank_line.is_match(line) || patterns.attribute_line.is_match(line) {
                continue;
            }
            if brace_delta > 0 {
                test_block_depth = Some(brace_delta);
                pending_test_item = false;
            } else if line.contains(';') {
                pending_test_item = false;
            }
            continue;
        }

        if patterns.test_attribute.is_match(line) {
            pending_test_item = true;
            continue;
        }

        if patterns.test_module.is_match(line) {
            test_block_depth = Some(brace_delta.max(1));
            continue;
        }

        // Comment lines accumulate allow rules for the code that follows.
        if patterns.comment_line.is_match(line) {
            if let Some(rule) = line_allow_rule(line) {
                pending_allow_rules.push(rule);
            }
            continue;
        }

        // A blank line between the comment and the code does not break the pairing.
        if patterns.blank_line.is_match(line) && !pending_allow_rules.is_empty() {
            continue;
        }

        let mut line_allow_rules: BTreeSet<String> = active_allow_rules.clone();
        line_allow_rules.extend(pending_allow_rules.iter().cloned());
        let starts_allowed_block = !pending_allow_rules.is_empty();
        pending_allow_rules.clear();

        let previous_line = if index == 0 { "" } else { lines[index - 1] };
        // A key whose literal starts on the next line is one statement wearing
        // two lines, and rustfmt writes it that way whenever the literal is
        // long. Match against both together so the rules see the statement
        // rather than half of it.
        let subject = join_wrapped_literal(line, lines.get(index + 1).copied());
        for (rule_id, message, regexes) in &patterns.rules {
            if !regexes.iter().any(|regex| regex.is_match(&subject)) {
                continue;
            }
            let violation = Violation {
                path: file_path.to_string(),
                line: index + 1,
                rule: (*rule_id).to_string(),
                message: (*message).to_string(),
                excerpt: subject.trim().to_string(),
            };
            let exempt = allows_rule(line, rule_id)
                || allows_rule(previous_line, rule_id)
                || rules_contain(&line_allow_rules, rule_id)
                || allowlist.iter().any(|entry| {
                    entry.path == violation.path
                        && entry.rule == violation.rule
                        && entry.excerpt == violation.excerpt
                });
            if !exempt {
                violations.push(violation);
            }
        }

        // Track the extent of a block opened by an allow-commented line.
        let delta = brace_delta;
        if let Some(depth) = active_allow_depth {
            let next = depth + delta;
            if next <= 0 {
                active_allow_depth = None;
                active_allow_rules.clear();
            } else {
                active_allow_depth = Some(next);
            }
        } else if starts_allowed_block && delta > 0 {
            active_allow_rules = line_allow_rules;
            active_allow_depth = Some(delta);
        }
    }

    violations
}

/// Renders the failure report written to stderr when the scan blocks a change.
pub fn render_report(policy: &PromptPolicy, violations: &[Violation]) -> String {
    let mut out = String::from("ai-prompt-trojan-guard blocked this change.\n");
    out.push_str(&format!(
        "All AI-facing static text must live in one of: {}.\n",
        policy.prompt_roots.join(", ")
    ));
    out.push_str("Each violation below is treated as a security and privacy defect:\n");
    for violation in violations {
        out.push_str(&format!(
            "- {}:{} [{}] {}\n  {}\n",
            violation.path, violation.line, violation.rule, violation.message, violation.excerpt
        ));
    }
    out
}

/// Prompt roots and source boundaries are declared by the consuming application.
pub struct PromptPolicy {
    prompt_roots: Vec<String>,
    sources: Vec<Regex>,
}
impl PromptPolicy {
    pub fn new(roots: &[&str], source_patterns: &[&str]) -> Result<Self> {
        anyhow::ensure!(!roots.is_empty(), "Declare at least one prompt root.");
        Ok(Self {
            prompt_roots: roots.iter().map(|root| (*root).into()).collect(),
            sources: source_patterns
                .iter()
                .map(|pattern| Regex::new(pattern))
                .collect::<std::result::Result<_, _>>()?,
        })
    }
    pub fn is_prompt_source(&self, path: &str) -> bool {
        self.prompt_roots
            .iter()
            .any(|root| path == root || (root.ends_with('/') && path.starts_with(root)))
    }
    pub fn is_candidate(&self, path: &str) -> bool {
        self.sources.iter().any(|pattern| pattern.is_match(path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn declared_roots_are_the_only_prompt_exemption_inside_the_source_boundary() {
        let policy = PromptPolicy::new(&["src/prompts/"], &[r"^src/.*$"]).unwrap();
        let source = r#"let prompt = "You are a careful assistant.";"#;
        assert!(scan_content(&policy, "src/prompts/request.rs", source, &[]).is_empty());
        assert!(!scan_content(&policy, "src/request.rs", source, &[]).is_empty());
        assert!(!policy.is_prompt_source("src/prompts-sibling/request.rs"));
    }
}
