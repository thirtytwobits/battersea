use battersea_flow::{FlowNode, FlowNodeDefinition};
const MIN_CAPACITY: u32 = 1;
/// Bounded character ring backing an `InMemoryBuffer` flow node.
///
/// The ring counts characters (Unicode scalar values), not bytes — the UI
/// measures characters and a byte-counted cap can split a multi-byte
/// codepoint at the boundary, producing invalid text. `dropped_chars` is a
/// monotonic counter of characters evicted since the buffer was created so
/// widgets can render a "log truncated" hint when useful.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "RingBufferState")]
pub struct RingBuffer {
    capacity: u32,
    text: String,
    dropped_chars: u64,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RingBufferState {
    capacity: u32,
    text: String,
    dropped_chars: u64,
}
impl TryFrom<RingBufferState> for RingBuffer {
    type Error = String;
    fn try_from(state: RingBufferState) -> Result<Self, Self::Error> {
        if state.capacity < MIN_CAPACITY || state.text.chars().count() > state.capacity as usize {
            return Err("Invalid ring buffer checkpoint capacity.".into());
        }
        Ok(Self {
            capacity: state.capacity,
            text: state.text,
            dropped_chars: state.dropped_chars,
        })
    }
}

impl RingBuffer {
    pub fn new(capacity: u32) -> Self {
        Self {
            capacity: capacity.max(MIN_CAPACITY),
            text: String::new(),
            dropped_chars: 0,
        }
    }

    /// Updates the configured capacity. When shrinking below the current
    /// occupancy, drops oldest characters to fit and bumps `dropped_chars`.
    pub fn set_capacity(&mut self, capacity: u32) {
        self.capacity = capacity.max(MIN_CAPACITY);
        let current_len = self.text.chars().count();
        if current_len > self.capacity as usize {
            let drop = current_len - self.capacity as usize;
            let tail: String = self.text.chars().skip(drop).collect();
            self.text = tail;
            self.dropped_chars = self.dropped_chars.saturating_add(drop as u64);
        }
    }

    pub fn append(&mut self, delta: &str) {
        if delta.is_empty() {
            return;
        }
        let cap = self.capacity as usize;
        let incoming_len = delta.chars().count();

        if incoming_len >= cap {
            // The new chunk alone overflows: drop everything we had plus
            // the prefix of `delta` that won't fit.
            let dropped_now = self.text.chars().count() + (incoming_len - cap);
            self.text = delta.chars().skip(incoming_len - cap).collect();
            self.dropped_chars = self.dropped_chars.saturating_add(dropped_now as u64);
            return;
        }

        let current_len = self.text.chars().count();
        let combined = current_len + incoming_len;
        if combined > cap {
            let drop = combined - cap;
            let mut tail: String = self.text.chars().skip(drop).collect();
            tail.push_str(delta);
            self.text = tail;
            self.dropped_chars = self.dropped_chars.saturating_add(drop as u64);
        } else {
            self.text.push_str(delta);
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn dropped_chars(&self) -> u64 {
        self.dropped_chars
    }
}

pub fn prepare_static_prompt(
    node: &FlowNode,
    definition: &FlowNodeDefinition,
) -> battersea_runtime::Token {
    let text = battersea_flow::ports::effective_parameter_value(node, definition, "text")
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_default();
    let text = battersea_flow::prompt_markdown::nest_markdown_headings(&text);

    battersea_runtime::Token {
        token_type: "prompt.fragment".to_string(),
        value: battersea_flow::prompt_markdown::prompt_fragment_value(
            text,
            battersea_flow::prompt_markdown::PromptFragmentEncoding::Markdown,
        ),
    }
}

pub fn activation_text(
    values: Option<&std::collections::HashMap<String, serde_json::Value>>,
    name: &str,
) -> crate::NodeResult<String> {
    values
        .and_then(|values| values.get(name))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            crate::NodeError::invalid_request(format!(
                "Flow activation requires a string {name} activation value."
            ))
        })
}

#[cfg(test)]
mod checkpoint_tests {
    use super::RingBuffer;

    #[test]
    fn restored_buffer_continues_the_same_unicode_capacity_and_drop_accounting() {
        let mut original = RingBuffer::new(5);
        original.append("abcdef猫犬");
        let mut restored: RingBuffer =
            serde_json::from_slice(&serde_json::to_vec(&original).unwrap()).unwrap();
        for delta in ["λ", "zebra", "界界"] {
            original.append(delta);
            restored.append(delta);
            assert_eq!(restored.text(), original.text());
            assert_eq!(restored.dropped_chars(), original.dropped_chars());
            assert!(restored.text().chars().count() <= restored.capacity() as usize);
        }
    }

    #[test]
    fn restoration_rejects_an_impossible_capacity_or_occupancy() {
        for value in [
            serde_json::json!({"capacity":0,"text":"","dropped_chars":0}),
            serde_json::json!({"capacity":1,"text":"猫犬","dropped_chars":0}),
        ] {
            assert!(serde_json::from_value::<RingBuffer>(value).is_err());
        }
    }
}
