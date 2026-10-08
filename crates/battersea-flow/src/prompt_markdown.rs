//! Markdown normalisation helpers for prompt-fragment composition.

use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptFragmentEncoding {
    PlainText,
    Markdown,
    Xml,
    Plain,
}

impl PromptFragmentEncoding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PlainText => "plaintext",
            Self::Markdown => "markdown",
            Self::Xml => "xml",
            Self::Plain => "plain",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "plaintext" => Some(Self::PlainText),
            "markdown" => Some(Self::Markdown),
            "xml" => Some(Self::Xml),
            "plain" => Some(Self::Plain),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptFragmentItem {
    pub text: String,
    pub encoding: Option<PromptFragmentEncoding>,
}

pub fn prompt_fragment_value(text: impl Into<String>, encoding: PromptFragmentEncoding) -> Value {
    json!({
        "text": text.into(),
        "encoding": encoding.as_str(),
    })
}

pub fn prompt_fragment_array_value(
    fragments: Vec<String>,
    encoding: PromptFragmentEncoding,
) -> Value {
    Value::Array(
        fragments
            .into_iter()
            .map(|fragment| prompt_fragment_value(fragment, encoding))
            .collect(),
    )
}

pub fn prompt_fragment_text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .or_else(|| prompt_fragment_item(value).map(|item| item.text))
}

pub fn prompt_fragment_items(value: &Value) -> Vec<PromptFragmentItem> {
    if let Some(item) = prompt_fragment_item(value) {
        return vec![item];
    }

    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(prompt_fragment_item)
        .collect()
}

fn prompt_fragment_item(value: &Value) -> Option<PromptFragmentItem> {
    if let Some(text) = value.as_str() {
        return Some(PromptFragmentItem {
            text: text.to_string(),
            encoding: None,
        });
    }

    let object = value.as_object()?;
    let text = object.get("text")?.as_str()?.to_string();
    let encoding = object
        .get("encoding")
        .and_then(Value::as_str)
        .and_then(PromptFragmentEncoding::from_str);
    Some(PromptFragmentItem { text, encoding })
}

/// Shifts authored markdown headings down one level so H1 remains available
/// for the containing prompt fragment's section heading.
pub fn nest_markdown_headings(markdown: &str) -> String {
    transform_markdown_lines(markdown, MarkdownLineTransform::NestHeadings)
}

/// Escapes heading-shaped plaintext lines before inserting them into markdown.
pub fn escape_plaintext_markdown_headings(text: &str) -> String {
    transform_markdown_lines(text, MarkdownLineTransform::EscapePlaintextHeadings)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MarkdownLineTransform {
    NestHeadings,
    EscapePlaintextHeadings,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Fence {
    marker: char,
    len: usize,
}

fn transform_markdown_lines(input: &str, transform: MarkdownLineTransform) -> String {
    let mut output = Vec::<String>::new();
    let mut fence: Option<Fence> = None;

    for raw_line in input.split_inclusive('\n') {
        let (line, ending) = split_line_ending(raw_line);

        if let Some(active_fence) = fence.as_ref() {
            if is_closing_fence(line, active_fence) {
                fence = None;
            }
            output.push(raw_line.to_string());
            continue;
        }

        if let Some(opening_fence) = opening_fence(line) {
            fence = Some(opening_fence);
            output.push(raw_line.to_string());
            continue;
        }

        if let Some(transformed) = transform_atx_heading_line(line, ending, transform) {
            output.push(transformed);
            continue;
        }

        if let Some(level) = setext_heading_level(line) {
            if let Some(previous) = output.last_mut() {
                let (previous_line, previous_ending) = split_line_ending(previous);
                if is_setext_heading_text(previous_line) {
                    match transform {
                        MarkdownLineTransform::NestHeadings => {
                            let nested_level = (level + 1).min(6);
                            *previous = format!(
                                "{} {}{}",
                                "#".repeat(nested_level),
                                previous_line.trim(),
                                previous_ending
                            );
                        }
                        MarkdownLineTransform::EscapePlaintextHeadings => {
                            output.push(escape_first_marker(line, ending));
                        }
                    }
                    if transform == MarkdownLineTransform::NestHeadings {
                        continue;
                    }
                    continue;
                }
            }
        }

        output.push(raw_line.to_string());
    }

    output.concat()
}

fn split_line_ending(raw_line: &str) -> (&str, &str) {
    if let Some(line) = raw_line.strip_suffix("\r\n") {
        (line, "\r\n")
    } else if let Some(line) = raw_line.strip_suffix('\n') {
        (line, "\n")
    } else {
        (raw_line, "")
    }
}

fn transform_atx_heading_line(
    line: &str,
    ending: &str,
    transform: MarkdownLineTransform,
) -> Option<String> {
    let indent = count_leading_spaces(line);
    if indent > 3 {
        return None;
    }
    let after_indent = &line[indent..];
    if after_indent.starts_with("\\#") {
        return None;
    }

    let heading_len = after_indent.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&heading_len) {
        return None;
    }

    let rest = &after_indent[heading_len..];
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }

    match transform {
        MarkdownLineTransform::NestHeadings if heading_len < 6 => {
            Some(format!("{}#{}{}", &line[..indent], after_indent, ending))
        }
        MarkdownLineTransform::NestHeadings => None,
        MarkdownLineTransform::EscapePlaintextHeadings => Some(escape_first_marker(line, ending)),
    }
}

fn count_leading_spaces(line: &str) -> usize {
    line.as_bytes()
        .iter()
        .take_while(|byte| **byte == b' ')
        .count()
}

fn opening_fence(line: &str) -> Option<Fence> {
    let indent = count_leading_spaces(line);
    if indent > 3 {
        return None;
    }
    let trimmed = &line[indent..];
    let marker = trimmed.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let len = trimmed.chars().take_while(|ch| *ch == marker).count();
    (len >= 3).then_some(Fence { marker, len })
}

fn is_closing_fence(line: &str, fence: &Fence) -> bool {
    let indent = count_leading_spaces(line);
    if indent > 3 {
        return false;
    }
    let trimmed = &line[indent..];
    let len = trimmed.chars().take_while(|ch| *ch == fence.marker).count();
    len >= fence.len && trimmed[len..].trim().is_empty()
}

fn setext_heading_level(line: &str) -> Option<usize> {
    let indent = count_leading_spaces(line);
    if indent > 3 {
        return None;
    }
    let trimmed = line[indent..].trim();
    if trimmed.is_empty() {
        return None;
    }
    let marker = trimmed.chars().next()?;
    if marker != '=' && marker != '-' {
        return None;
    }
    trimmed
        .chars()
        .all(|ch| ch == marker)
        .then_some(if marker == '=' { 1 } else { 2 })
}

fn is_setext_heading_text(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    transform_atx_heading_line(line, "", MarkdownLineTransform::NestHeadings).is_none()
        && opening_fence(line).is_none()
}

fn escape_first_marker(line: &str, ending: &str) -> String {
    let indent = count_leading_spaces(line);
    format!("{}\\{}{}", &line[..indent], &line[indent..], ending)
}

#[cfg(test)]
mod tests {
    use super::{escape_plaintext_markdown_headings, nest_markdown_headings};

    #[test]
    fn nest_markdown_headings_shifts_atx_and_setext_headings() {
        let input = "# Title\n\nSection\n---\n\n## Detail\n";

        assert_eq!(
            nest_markdown_headings(input),
            "## Title\n\n### Section\n\n### Detail\n"
        );
    }

    #[test]
    fn nest_markdown_headings_ignores_fenced_code_and_escaped_markers() {
        let input = "\\# Literal\n\n```md\n# Code\n```\n\n# Real\n";

        assert_eq!(
            nest_markdown_headings(input),
            "\\# Literal\n\n```md\n# Code\n```\n\n## Real\n"
        );
    }

    #[test]
    fn nest_markdown_headings_leaves_h6_valid() {
        assert_eq!(nest_markdown_headings("###### Deep\n"), "###### Deep\n");
    }

    #[test]
    fn escape_plaintext_markdown_headings_protects_heading_shapes() {
        let input = "# Literal\n\nTitle\n---\n\n  ## Also literal\n\nFinal setext\n---";

        assert_eq!(
            escape_plaintext_markdown_headings(input),
            "\\# Literal\n\nTitle\n\\---\n\n  \\## Also literal\n\nFinal setext\n\\---"
        );
    }
}
