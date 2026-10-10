//! Copyright (c) Scott A Dixon
use anyhow::{anyhow, Result};
use battersea_model::{adapter::*, engine::*, media::*};
use serde::Deserialize;
use std::collections::HashMap;
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineBackendConfigFile {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(rename = "displayOrder", default)]
    pub display_order: Option<u32>,
    #[serde(rename = "contextWindowTokens", default)]
    pub context_window_tokens: Option<u64>,
    pub capabilities: EngineBackendCapabilities,
    #[serde(default)]
    pub properties: Option<EngineChatConfigFile>,
    #[serde(default)]
    pub options: Option<EngineBackendOptions>,
    #[serde(default)]
    pub auth: Option<EngineAuthConfigFile>,
    #[serde(default)]
    pub short_description: Option<String>,
    #[serde(default)]
    pub long_description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EngineChatConfigFile {
    pub response_format: Option<battersea_model::ResponseFormat>,
    pub output_modalities: Option<Vec<battersea_model::Modality>>,
    #[serde(default)]
    pub stream: Option<bool>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub top_k: Option<u32>,
    #[serde(default)]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(default)]
    pub tool_execution: Option<EngineToolExecutionMode>,
    #[serde(default)]
    pub max_tool_rounds: Option<u32>,
    #[serde(default)]
    pub tool_choice: Option<EngineChatToolChoice>,
    #[serde(default)]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default)]
    pub strict_tool_inputs: Option<bool>,
    #[serde(default)]
    pub service_tier: Option<String>,
    #[serde(default)]
    pub safety_identifier: Option<String>,
    #[serde(default)]
    pub request_metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub reasoning_effort: Option<EngineReasoningEffort>,
    #[serde(default)]
    pub reasoning_summary: Option<EngineReasoningSummary>,
    #[serde(default)]
    pub thinking_budget_tokens: Option<u32>,
    #[serde(default)]
    pub thinking_visibility: Option<EngineThinkingVisibility>,
    #[serde(default)]
    pub response_verbosity: Option<EngineResponseVerbosity>,
    #[serde(default)]
    pub context_overflow: Option<EngineContextOverflow>,
    #[serde(default)]
    pub prompt_cache_key: Option<String>,
    #[serde(default)]
    pub prompt_cache_retention: Option<EnginePromptCacheRetention>,
    #[serde(default)]
    pub store_response: Option<bool>,
    #[serde(default)]
    pub max_provider_tool_calls: Option<u32>,
    #[serde(default)]
    pub logprobs: Option<bool>,
    #[serde(default)]
    pub top_logprobs: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MediaBackendConfigFile {
    #[serde(default)]
    pub provider: String,
    pub capability: MediaCapability,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    pub capabilities: MediaBackendCapabilities,
    #[serde(default)]
    pub options: Option<EngineBackendOptions>,
    #[serde(default)]
    pub auth: Option<EngineAuthConfigFile>,
    #[serde(default)]
    pub short_description: Option<String>,
    #[serde(default)]
    pub long_description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct EngineAuthConfigFile {
    #[serde(default)]
    pub r#type: Option<String>,
    #[serde(rename = "apiKey", default)]
    pub api_key: Option<String>,
    #[serde(rename = "apiKeyEnv", default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(rename = "versionHeader", default)]
    pub version_header: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
}

pub fn default_true() -> bool {
    true
}
pub fn normalize_backends(
    configured: &HashMap<String, EngineBackendConfigFile>,
) -> Result<HashMap<String, EngineBackendConfig>> {
    configured
        .iter()
        .map(|(id, backend)| Ok((id.clone(), merge_backend(id, backend.clone())?)))
        .collect()
}
pub fn normalize_media_backends(
    configured: &HashMap<String, MediaBackendConfigFile>,
) -> HashMap<String, MediaBackendConfig> {
    configured
        .iter()
        .map(|(id, backend)| (id.clone(), merge_media_backend(id, backend.clone())))
        .collect()
}
pub fn make_auth(
    auth_type: &str,
    api_key: Option<&str>,
    api_key_env: &str,
    header: Option<&str>,
    version_header: Option<&str>,
    version: Option<&str>,
) -> battersea_model::adapter::EngineAuthConfig {
    let api_key = api_key
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            if api_key_env.trim().is_empty() {
                None
            } else {
                std::env::var(api_key_env)
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            }
        });
    battersea_model::adapter::EngineAuthConfig {
        auth_type: auth_type.to_string(),
        api_key_env: api_key_env.to_string(),
        header: header.map(ToOwned::to_owned),
        version_header: version_header.map(ToOwned::to_owned),
        version: version.map(ToOwned::to_owned),
        has_api_key: api_key.is_some(),
        api_key,
    }
}
pub fn merge_media_backend(id: &str, configured: MediaBackendConfigFile) -> MediaBackendConfig {
    let auth = configured.auth.unwrap_or_default();

    MediaBackendConfig {
        id: id.to_string(),
        provider: configured.provider,
        capability: configured.capability,
        label: configured.label.unwrap_or_else(|| id.to_string()),
        enabled: configured.enabled,
        endpoint: configured.endpoint.unwrap_or_default(),
        model: configured.model.unwrap_or_default(),
        capabilities: configured.capabilities,
        options: configured.options.unwrap_or_default(),
        auth: make_auth(
            auth.r#type.as_deref().unwrap_or("none"),
            auth.api_key.as_deref(),
            auth.api_key_env.as_deref().unwrap_or(""),
            auth.header.as_deref(),
            auth.version_header.as_deref(),
            auth.version.as_deref(),
        ),
        short_description: configured.short_description.unwrap_or_default(),
        long_description: configured.long_description.unwrap_or_default(),
    }
}
pub fn merge_backend(id: &str, configured: EngineBackendConfigFile) -> Result<EngineBackendConfig> {
    let provider = configured.provider;
    let auth = configured.auth.unwrap_or_default();

    let properties = configured.properties.ok_or_else(|| {
        anyhow!("Engine backend \"{id}\" must declare a chat `properties` block.")
    })?;
    let chat = resolve_chat_parameters(properties).map_err(|message| {
        anyhow!("Engine backend \"{id}\" has invalid properties config: {message}")
    })?;

    configured
        .capabilities
        .validate_chat_parameters(&chat)
        .map_err(|message| {
            anyhow!("Engine backend \"{id}\" has invalid chat parameters: {message}")
        })?;

    configured
        .capabilities
        .validate_reasoning_effort(configured.capabilities.reasoning_effort_when_unset)
        .map_err(|message| {
            anyhow!("Engine backend \"{id}\" has invalid reasoning capabilities: {message}")
        })?;

    Ok(EngineBackendConfig {
        id: id.to_string(),
        provider: provider.clone(),
        label: configured.label.unwrap_or_else(|| id.to_string()),
        enabled: configured.enabled,
        endpoint: configured.endpoint.unwrap_or_default(),
        model: configured.model.unwrap_or_default(),
        display_order: configured.display_order,
        chat,
        capabilities: configured.capabilities,
        context_window_tokens: configured
            .context_window_tokens
            .ok_or_else(|| anyhow!("Engine backend \"{id}\" must declare contextWindowTokens."))?,
        options: configured.options.unwrap_or_default(),
        auth: make_auth(
            auth.r#type.as_deref().unwrap_or("none"),
            auth.api_key.as_deref(),
            auth.api_key_env.as_deref().unwrap_or(""),
            auth.header.as_deref(),
            auth.version_header.as_deref(),
            auth.version.as_deref(),
        ),
        short_description: configured.short_description.unwrap_or_default(),
        long_description: configured.long_description.unwrap_or_default(),
    })
}
pub fn resolve_chat_parameters(
    configured: EngineChatConfigFile,
) -> std::result::Result<EngineChatParameters, String> {
    fn required<T>(value: Option<T>, name: &str) -> std::result::Result<T, String> {
        value.ok_or_else(|| format!("chat property \"{name}\" is required but was not configured"))
    }

    let chat = EngineChatParameters {
        response_format: configured.response_format,
        output_modalities: configured.output_modalities,
        stream: required(configured.stream, "stream")?,
        max_output_tokens: configured.max_output_tokens,
        temperature: required(configured.temperature, "temperature")?,
        top_p: configured.top_p,
        top_k: configured.top_k,
        stop_sequences: required(configured.stop_sequences, "stopSequences")?,
        tool_execution: required(configured.tool_execution, "toolExecution")?,
        max_tool_rounds: required(configured.max_tool_rounds, "maxToolRounds")?,
        tool_choice: required(configured.tool_choice, "toolChoice")?,
        parallel_tool_calls: configured.parallel_tool_calls,
        strict_tool_inputs: configured.strict_tool_inputs,
        service_tier: configured.service_tier.and_then(non_empty_string),
        safety_identifier: configured.safety_identifier.and_then(non_empty_string),
        request_metadata: configured.request_metadata,
        reasoning_effort: configured.reasoning_effort,
        reasoning_summary: configured.reasoning_summary,
        thinking_budget_tokens: configured.thinking_budget_tokens,
        thinking_visibility: configured.thinking_visibility,
        response_verbosity: configured.response_verbosity,
        context_overflow: required(configured.context_overflow, "contextOverflow")?,
        prompt_cache_key: configured.prompt_cache_key.and_then(non_empty_string),
        prompt_cache_retention: configured.prompt_cache_retention,
        store_response: configured.store_response,
        max_provider_tool_calls: configured.max_provider_tool_calls,
        logprobs: configured.logprobs,
        top_logprobs: configured.top_logprobs,
    };
    validate_chat_parameters(&chat)?;
    Ok(chat)
}
pub fn merge_chat_parameters(
    // The `provider` parameter is kept on the signature so callers
    // don't have to thread a different shape through; the only
    // historical user (provider-specific tool-execution fallbacks)
    // was removed when `engine-orchestrated` became the sole
    // supported mode. Underscore-prefixed to silence unused-arg
    // warnings until a future refactor needs it again.
    _provider: &str,
    mut chat: EngineChatParameters,
    configured: EngineChatConfigFile,
) -> std::result::Result<EngineChatParameters, String> {
    if let Some(format) = configured.response_format {
        chat.response_format = Some(format);
    }
    if let Some(modalities) = configured.output_modalities {
        chat.output_modalities = Some(modalities);
    }
    if let Some(stream) = configured.stream {
        chat.stream = stream;
    }
    if let Some(max_output_tokens) = configured.max_output_tokens {
        chat.max_output_tokens = Some(max_output_tokens);
    }
    if let Some(temperature) = configured.temperature {
        chat.temperature = temperature;
    }
    if let Some(top_p) = configured.top_p {
        chat.top_p = Some(top_p);
    }
    if let Some(top_k) = configured.top_k {
        chat.top_k = Some(top_k);
    }
    if let Some(stop_sequences) = configured.stop_sequences {
        chat.stop_sequences = stop_sequences;
    }
    if let Some(tool_execution) = configured.tool_execution {
        chat.tool_execution = tool_execution;
    }
    if let Some(max_tool_rounds) = configured.max_tool_rounds {
        chat.max_tool_rounds = max_tool_rounds;
    }
    if let Some(tool_choice) = configured.tool_choice {
        chat.tool_choice = tool_choice;
    }
    if let Some(value) = configured.parallel_tool_calls {
        chat.parallel_tool_calls = Some(value);
    }
    if let Some(value) = configured.strict_tool_inputs {
        chat.strict_tool_inputs = Some(value);
    }
    if let Some(value) = configured.service_tier {
        chat.service_tier = non_empty_string(value);
    }
    if let Some(value) = configured.safety_identifier {
        chat.safety_identifier = non_empty_string(value);
    }
    if let Some(value) = configured.request_metadata {
        chat.request_metadata = Some(value);
    }
    if let Some(value) = configured.reasoning_effort {
        chat.reasoning_effort = Some(value);
    }
    if let Some(value) = configured.reasoning_summary {
        chat.reasoning_summary = Some(value);
    }
    if let Some(value) = configured.thinking_budget_tokens {
        chat.thinking_budget_tokens = Some(value);
    }
    if let Some(value) = configured.thinking_visibility {
        chat.thinking_visibility = Some(value);
    }
    if let Some(value) = configured.response_verbosity {
        chat.response_verbosity = Some(value);
    }
    if let Some(value) = configured.context_overflow {
        chat.context_overflow = value;
    }
    if let Some(value) = configured.prompt_cache_key {
        chat.prompt_cache_key = non_empty_string(value);
    }
    if let Some(value) = configured.prompt_cache_retention {
        chat.prompt_cache_retention = Some(value);
    }
    if let Some(value) = configured.store_response {
        chat.store_response = Some(value);
    }
    if let Some(value) = configured.max_provider_tool_calls {
        chat.max_provider_tool_calls = Some(value);
    }
    if let Some(value) = configured.logprobs {
        chat.logprobs = Some(value);
    }
    if let Some(value) = configured.top_logprobs {
        chat.top_logprobs = Some(value);
    }
    validate_chat_parameters(&chat)?;
    Ok(chat)
}
pub fn non_empty_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}
pub fn validate_chat_parameters(chat: &EngineChatParameters) -> std::result::Result<(), String> {
    if chat.max_output_tokens == Some(0) {
        return Err("maxOutputTokens must be greater than 0".to_string());
    }
    if !(0.0..=2.0).contains(&chat.temperature) || !chat.temperature.is_finite() {
        return Err("temperature must be a finite value between 0 and 2".to_string());
    }
    if let Some(top_p) = chat.top_p {
        if !(0.0..=1.0).contains(&top_p) || !top_p.is_finite() {
            return Err("topP must be a finite value between 0 and 1".to_string());
        }
    }
    if matches!(chat.top_k, Some(0)) {
        return Err("topK must be greater than 0 when set".to_string());
    }
    if !(1..=32).contains(&chat.max_tool_rounds) {
        return Err("maxToolRounds must be between 1 and 32".to_string());
    }
    if chat.tool_choice.mode == EngineChatToolChoiceMode::Tool
        && chat
            .tool_choice
            .tool_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
    {
        return Err("toolChoice.toolName is required when toolChoice.mode is tool".to_string());
    }
    if matches!(chat.thinking_budget_tokens, Some(0)) {
        return Err("thinkingBudgetTokens must be greater than 0 when set".to_string());
    }
    if matches!(chat.max_provider_tool_calls, Some(0)) {
        return Err("maxProviderToolCalls must be greater than 0 when set".to_string());
    }
    if matches!(chat.top_logprobs, Some(0)) {
        return Err("topLogprobs must be greater than 0 when set".to_string());
    }
    Ok(())
}
/// Adds the library-owned backend definitions to an application schema.
pub fn compose_backend_schema(mut application: serde_json::Value) -> Result<serde_json::Value> {
    let generic: serde_json::Value = serde_yaml::from_str(include_str!("backend.schema.yaml"))?;
    let definitions = application
        .as_object_mut()
        .ok_or_else(|| anyhow!("Application schema must be an object."))?
        .entry("$defs")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| anyhow!("$defs must be an object."))?;
    for (name, definition) in generic["$defs"].as_object().expect("bundled definitions") {
        anyhow::ensure!(
            !definitions.contains_key(name),
            "Application redefines library schema {name}."
        );
        definitions.insert(name.clone(), definition.clone());
    }
    Ok(application)
}

/// Expands YAML merge keys (`<<: *anchor`) recursively across a value
/// tree. serde_yaml does not honour merge keys natively (neither for
/// `Value` parsing nor for `Deserialize`-driven struct decoding), so
/// the engine config layer applies the YAML 1.1 merge-key rules
/// itself: parent keys are inherited, child keys win on conflict, and
/// a sequence-valued `<<` is treated as multiple parents merged
/// left-to-right with later parents losing to earlier ones (matching
/// the standard).
///
/// Without this pass, the family-base anchors that engine.yaml uses
/// to share provider/auth/properties defaults would surface as
/// literal `<<` keys in the validator and trip
/// `additionalProperties: false`.
///
/// **Special case for `long_description`:** unlike other fields where
/// the child overrides the parent, `long_description` ACCUMULATES
/// across the merge chain. Every parent in declaration order
/// contributes its section first, then the child appends its own,
/// joined by a blank line. Authors compose backend documentation as
/// a stack of markdown sections (one per family contribution + one
/// concrete section) rather than retyping shared context per
/// concrete entry.
pub fn resolve_yaml_merge_keys(value: serde_yaml::Value) -> serde_yaml::Value {
    use serde_yaml::Mapping;

    match value {
        serde_yaml::Value::Mapping(map) => {
            let merge_marker = serde_yaml::Value::String("<<".to_string());
            let mut resolved = Mapping::new();

            // First, walk parent merges. The YAML 1.1 spec gives the
            // EARLIEST-listed parent the highest precedence among
            // parents (later parents lose on conflict), so we apply
            // them in reverse-iteration order then let the child
            // overwrite both.
            let mut child_entries = Vec::new();
            let mut parent_blocks: Vec<serde_yaml::Value> = Vec::new();
            for (key, val) in map {
                if key == merge_marker {
                    let resolved_parent = resolve_yaml_merge_keys(val);
                    match resolved_parent {
                        serde_yaml::Value::Sequence(parents) => {
                            // Sequence form: `<<: [*a, *b]` — `*a`
                            // wins over `*b` on conflict. Push them
                            // in spec order; we apply in reverse
                            // below so first-listed wins.
                            parent_blocks.extend(parents);
                        }
                        other => parent_blocks.push(other),
                    }
                } else {
                    child_entries.push((key, val));
                }
            }

            // Pre-compute the accumulated long_description across the
            // merge chain BEFORE the standard child-wins overwrite
            // path runs. Parents contribute their sections in
            // declaration order; the child's own section (if any)
            // appends last. Empty / whitespace-only contributions
            // drop out so a family that doesn't author docs doesn't
            // leave a blank line in the assembled text.
            let accumulated_long_description =
                assemble_long_description_sections(&parent_blocks, &child_entries);

            for parent in parent_blocks.into_iter().rev() {
                if let serde_yaml::Value::Mapping(parent_map) = parent {
                    for (k, v) in parent_map {
                        resolved.insert(k, v);
                    }
                }
            }
            // Child wins last for everything except long_description,
            // which is replaced below by the accumulated value.
            for (k, v) in child_entries {
                resolved.insert(k, resolve_yaml_merge_keys(v));
            }
            // Whatever standard-merge ended up writing for
            // `long_description` is wrong — overwrite or drop it
            // based on the accumulated sections.
            let key = serde_yaml::Value::String("long_description".to_string());
            if let Some(combined) = accumulated_long_description {
                resolved.insert(key, serde_yaml::Value::String(combined));
            } else {
                resolved.remove(&key);
            }
            serde_yaml::Value::Mapping(resolved)
        }
        serde_yaml::Value::Sequence(seq) => {
            serde_yaml::Value::Sequence(seq.into_iter().map(resolve_yaml_merge_keys).collect())
        }
        other => other,
    }
}

/// Assembles a node's `long_description` from every parent block plus
/// the child's own entry, in declaration order. Each non-empty
/// contribution becomes one section in the joined result; empty
/// strings (and missing keys) are skipped silently so a parent that
/// doesn't author documentation doesn't leave a blank gap. Returns
/// None when the entire chain has nothing to say so callers know not
/// to insert an empty key.
fn assemble_long_description_sections(
    parent_blocks: &[serde_yaml::Value],
    child_entries: &[(serde_yaml::Value, serde_yaml::Value)],
) -> Option<String> {
    let key = serde_yaml::Value::String("long_description".to_string());
    let mut sections = Vec::<String>::new();
    for parent in parent_blocks {
        if let serde_yaml::Value::Mapping(parent_map) = parent {
            if let Some(serde_yaml::Value::String(text)) = parent_map.get(&key) {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    sections.push(trimmed.to_string());
                }
            }
        }
    }
    for (k, v) in child_entries {
        if k == &key {
            if let serde_yaml::Value::String(text) = v {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    sections.push(trimmed.to_string());
                }
            }
        }
    }
    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resolve_yaml_merge_keys_handles_single_parent_overrides_and_sequence_merges() {
        let yaml = r#"
families:
  base: &base
    a: 1
    b: 2
    nested:
      shared: yes
      base_only: true
  override: &override
    a: 999
    c: 3

backends:
  single-parent:
    <<: *base
    b: 200          # child wins
    nested:
      shared: no   # child wins inside nested
      child_only: y

  multi-parent:
    <<: [*override, *base]   # first-listed wins
    d: 4
"#;
        let raw: serde_yaml::Value = serde_yaml::from_str(yaml).expect("parse");
        let resolved = resolve_yaml_merge_keys(raw);
        let backends = resolved
            .get("backends")
            .expect("backends key")
            .as_mapping()
            .expect("backends mapping");

        let single = backends
            .get(serde_yaml::Value::String("single-parent".to_string()))
            .expect("single-parent")
            .as_mapping()
            .unwrap();
        // Inherited from *base
        assert_eq!(
            single
                .get(serde_yaml::Value::String("a".to_string()))
                .unwrap(),
            &serde_yaml::Value::Number(serde_yaml::Number::from(1))
        );
        // Child overrides the inherited b.
        assert_eq!(
            single
                .get(serde_yaml::Value::String("b".to_string()))
                .unwrap(),
            &serde_yaml::Value::Number(serde_yaml::Number::from(200))
        );
        // The nested mapping is REPLACED whole by the child
        // (consistent with YAML 1.1 — merging is shallow at the level
        // the merge key lives), and the resolver still walks into
        // nested mappings to expand any merge keys they carry.
        let nested = single
            .get(serde_yaml::Value::String("nested".to_string()))
            .and_then(|v| v.as_mapping())
            .expect("nested map");
        assert_eq!(
            nested
                .get(serde_yaml::Value::String("shared".to_string()))
                .unwrap(),
            &serde_yaml::Value::String("no".to_string())
        );
        assert!(nested
            .get(serde_yaml::Value::String("base_only".to_string()))
            .is_none());

        let multi = backends
            .get(serde_yaml::Value::String("multi-parent".to_string()))
            .expect("multi-parent")
            .as_mapping()
            .unwrap();
        // First-listed parent (*override) wins over later (*base) on
        // conflict — `a` resolves to 999, not 1.
        assert_eq!(
            multi
                .get(serde_yaml::Value::String("a".to_string()))
                .unwrap(),
            &serde_yaml::Value::Number(serde_yaml::Number::from(999))
        );
        // Non-conflicting fields from both parents are present.
        assert_eq!(
            multi
                .get(serde_yaml::Value::String("b".to_string()))
                .unwrap(),
            &serde_yaml::Value::Number(serde_yaml::Number::from(2))
        );
        assert_eq!(
            multi
                .get(serde_yaml::Value::String("c".to_string()))
                .unwrap(),
            &serde_yaml::Value::Number(serde_yaml::Number::from(3))
        );
        // Plus the child's own fields.
        assert_eq!(
            multi
                .get(serde_yaml::Value::String("d".to_string()))
                .unwrap(),
            &serde_yaml::Value::Number(serde_yaml::Number::from(4))
        );

        // The `<<` key itself never survives into the resolved tree.
        assert!(single
            .get(serde_yaml::Value::String("<<".to_string()))
            .is_none());
        assert!(multi
            .get(serde_yaml::Value::String("<<".to_string()))
            .is_none());
    }
    #[test]
    fn resolve_yaml_merge_keys_accumulates_long_description_across_merge_chain() {
        let yaml = r#"
families:
  base: &base
    long_description: |
      ## Shared section
      Common defaults.
  flavor: &flavor
    <<: *base
    long_description: |
      ## Flavor section
      Adds flavor-specific defaults.
  silent: &silent
    long_description: ""
  also-silent: &also-silent
    other_field: "yes"

backends:
  child:
    <<: *flavor
    long_description: |
      ## Concrete model
      Per-model context.

  multi-parent:
    <<: [*flavor, *silent, *also-silent]
    long_description: |
      ## Multi-parent
      Child wins on ordering, every parent contributes.

  no-child-section:
    <<: *flavor
    label: noop

  silent-chain:
    <<: *silent
    long_description: ""
"#;
        let raw: serde_yaml::Value = serde_yaml::from_str(yaml).expect("parse");
        let resolved = resolve_yaml_merge_keys(raw);
        let backends = resolved
            .get("backends")
            .expect("backends")
            .as_mapping()
            .expect("backends mapping");

        // Single-parent chain: base contributes "Shared section", flavor
        // appends "Flavor section" while resolving its own merge,
        // then the concrete child appends "Concrete model". The
        // assembled value is all three joined by a blank line.
        let child = backends
            .get(serde_yaml::Value::String("child".to_string()))
            .and_then(|v| v.as_mapping())
            .expect("child");
        let child_text = child
            .get(serde_yaml::Value::String("long_description".to_string()))
            .and_then(|v| v.as_str())
            .expect("child long_description");
        assert!(
            child_text.contains("## Shared section"),
            "expected base section, got:\n{child_text}"
        );
        assert!(
            child_text.contains("## Flavor section"),
            "expected flavor section, got:\n{child_text}"
        );
        assert!(
            child_text.contains("## Concrete model"),
            "expected concrete section, got:\n{child_text}"
        );
        // Sections appear in declaration order: base → flavor → concrete.
        let base_pos = child_text.find("## Shared section").unwrap();
        let flavor_pos = child_text.find("## Flavor section").unwrap();
        let concrete_pos = child_text.find("## Concrete model").unwrap();
        assert!(base_pos < flavor_pos, "base must precede flavor");
        assert!(flavor_pos < concrete_pos, "flavor must precede concrete");
        // Sections are joined by a blank line.
        assert!(child_text.contains("Common defaults.\n\n## Flavor section"));

        // Multi-parent: silent and also-silent contribute nothing
        // (one has empty long_description, the other has none) so
        // they don't introduce blank lines into the assembled text.
        let multi = backends
            .get(serde_yaml::Value::String("multi-parent".to_string()))
            .and_then(|v| v.as_mapping())
            .expect("multi-parent");
        let multi_text = multi
            .get(serde_yaml::Value::String("long_description".to_string()))
            .and_then(|v| v.as_str())
            .expect("multi long_description");
        assert!(multi_text.contains("## Shared section"));
        assert!(multi_text.contains("## Flavor section"));
        assert!(multi_text.contains("## Multi-parent"));
        // No double-blank-line gaps from the silent parents.
        assert!(
            !multi_text.contains("\n\n\n"),
            "silent parents should not insert empty sections, got:\n{multi_text}"
        );

        // A concrete backend that doesn't author its own section still
        // inherits the parent chain's accumulated text.
        let no_child = backends
            .get(serde_yaml::Value::String("no-child-section".to_string()))
            .and_then(|v| v.as_mapping())
            .expect("no-child-section");
        let no_child_text = no_child
            .get(serde_yaml::Value::String("long_description".to_string()))
            .and_then(|v| v.as_str())
            .expect("no-child long_description");
        assert!(no_child_text.contains("## Shared section"));
        assert!(no_child_text.contains("## Flavor section"));
        assert!(!no_child_text.contains("## Concrete model"));

        // When the entire chain has nothing to say (silent parent +
        // empty child), `long_description` is omitted entirely from
        // the resolved tree rather than left as an empty string.
        let silent = backends
            .get(serde_yaml::Value::String("silent-chain".to_string()))
            .and_then(|v| v.as_mapping())
            .expect("silent-chain");
        assert!(silent
            .get(serde_yaml::Value::String("long_description".to_string()))
            .is_none());
    }
    #[test]
    fn schema_composition_preserves_application_contract_and_rejects_library_redefinition() {
        let application = serde_json::json!({"type":"object","$defs":{"product":{"type":"string"}},"properties":{"value":{"$ref":"#/$defs/product"}}});
        let composed = compose_backend_schema(application.clone()).unwrap();
        assert_eq!(composed["properties"], application["properties"]);
        assert_eq!(
            composed["$defs"]["product"],
            application["$defs"]["product"]
        );
        assert!(composed["$defs"]["mediaBackend"].is_object());
        assert!(compose_backend_schema(composed).is_err());
    }
}
