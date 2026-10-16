//! Versioned declarative provider configuration. Parsing and validation perform no provider I/O.
use crate::{EngineBackendConfigFile, MediaBackendConfigFile};
use battersea_model::media::*;
use battersea_model::*;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

pub const PROVIDER_CONFIGURATION_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProtocolBindingFile {
    pub adapter: String,
    pub base_url: String,
    pub auth: Authentication,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub delivery_origins: Vec<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<ContentCapabilities>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Vec<String>>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counting: Option<TokenCounting>,
}
impl ProtocolBindingFile {
    fn connection(&self) -> ProviderConnection {
        ProviderConnection {
            base_url: self.base_url.clone(),
            auth: self.auth.clone(),
            headers: self.headers.clone(),
            delivery_origins: self.delivery_origins.clone(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ProviderFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat: Option<ProtocolBindingFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media: Option<ProtocolBindingFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderConfiguration {
    pub version: u32,
    #[serde(deserialize_with = "unique_map")]
    pub providers: BTreeMap<String, ProviderFile>,
    #[serde(default, deserialize_with = "unique_map")]
    pub backends: BTreeMap<String, EngineBackendConfigFile>,
    #[serde(default, deserialize_with = "unique_map")]
    pub media_backends: BTreeMap<String, MediaBackendConfigFile>,
}
fn unique_map<'de, D, T>(deserializer: D) -> Result<BTreeMap<String, T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct Visitor<T>(std::marker::PhantomData<T>);
    impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Visitor<T> {
        type Value = BTreeMap<String, T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("unique non-blank identities")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut out = BTreeMap::new();
            while let Some((id, value)) = map.next_entry::<String, T>()? {
                if id.is_empty() || id.trim() != id || out.insert(id, value).is_some() {
                    return Err(serde::de::Error::custom("duplicate or blank identity"));
                }
            }
            Ok(out)
        }
    }
    deserializer.deserialize_map(Visitor(std::marker::PhantomData))
}

fn error(
    code: ProviderCode,
    path: &str,
    provider: &str,
    adapter: &str,
) -> ProviderConfigurationError {
    ProviderConfigurationError::new(code, path).at_binding(provider, adapter)
}
fn options(
    adapter: &ProtocolAdapter,
    options: &EngineBackendOptions,
) -> Result<(), ProviderConfigurationError> {
    if options.max_retries.unwrap_or(0) > 0
        && !matches!((options.retry_initial_delay_ms,options.retry_max_delay_ms),(Some(a),Some(b)) if a > 0 && b >= a)
    {
        return Err(ProviderConfigurationError::new(
            ProviderCode::InvalidOptions,
            "options.retryMaxDelayMs",
        ));
    }
    let mut declared = options.clone();
    declared.extra.clear();
    let mut value = serde_json::to_value(&declared).expect("options serialise");
    let object = value.as_object_mut().expect("options object");
    if options.extra.keys().any(|key| object.contains_key(key)) {
        return Err(ProviderConfigurationError::new(
            ProviderCode::InvalidOptions,
            "options",
        ));
    }
    object.retain(|_, v| !v.is_null());
    object.extend(options.extra.clone());
    if !jsonschema::validator_for(&adapter.options_schema)
        .expect("registered schema")
        .is_valid(&value)
    {
        return Err(ProviderConfigurationError::new(
            ProviderCode::InvalidOptions,
            "options",
        ));
    }
    Ok(())
}

/// A validated declaration owns the exact adapter registrations used for binding.
pub struct ValidatedProviderConfiguration {
    configuration: ProviderConfiguration,
    adapters: BTreeMap<(String, String), Arc<ProtocolAdapter>>,
}
impl ProviderConfiguration {
    pub fn validate(
        &self,
        protocols: &ProtocolRegistry,
    ) -> Result<ValidatedProviderConfiguration, ProviderConfigurationError> {
        if self.version != PROVIDER_CONFIGURATION_VERSION {
            return Err(ProviderConfigurationError::new(
                ProviderCode::UnsupportedVersion,
                "version",
            ));
        }
        let mut configuration = self.clone();
        let mut adapters = BTreeMap::new();
        let mut registry = ProviderRegistry::default();
        for (id, provider) in &mut configuration.providers {
            if id.is_empty() || id.trim() != id {
                return Err(ProviderConfigurationError::new(
                    ProviderCode::DuplicateIdentity,
                    "providers",
                ));
            }
            if provider.chat.is_none() && provider.media.is_none() {
                return Err(error(
                    ProviderCode::UnsupportedOperation,
                    "providers",
                    id,
                    "",
                ));
            }
            for (kind, binding) in [("chat", &mut provider.chat), ("media", &mut provider.media)] {
                let Some(binding) = binding else { continue };
                let base = format!("providers.{id}.{kind}");
                let adapter = protocols
                    .adapter(&binding.adapter)
                    .map_err(|e| error(e.code, &base, id, &binding.adapter))?;
                let connection = binding.connection().normalise().map_err(|e| {
                    error(e.code, &format!("{base}.{}", e.path), id, &binding.adapter)
                })?;
                if adapter.requires_endpoint && connection.base_url.is_empty() {
                    return Err(error(
                        ProviderCode::InvalidEndpoint,
                        &base,
                        id,
                        &binding.adapter,
                    ));
                }
                for header in &adapter.required_headers {
                    if !connection.headers.contains_key(header) {
                        return Err(error(
                            ProviderCode::InvalidAuthentication,
                            &format!("{base}.headers"),
                            id,
                            &binding.adapter,
                        ));
                    }
                }
                binding.base_url = connection.base_url;
                binding.auth = connection.auth;
                binding.headers = connection.headers;
                binding.delivery_origins = connection.delivery_origins;
                if binding
                    .counting
                    .is_some_and(|c| c != TokenCounting::Unavailable && c != adapter.counting)
                {
                    return Err(error(
                        ProviderCode::UnsupportedCapability,
                        &format!("{base}.counting"),
                        id,
                        &binding.adapter,
                    ));
                }
                if kind == "chat" {
                    let mut chat = adapter.chat.clone().ok_or_else(|| {
                        error(
                            ProviderCode::UnsupportedOperation,
                            &base,
                            id,
                            &binding.adapter,
                        )
                    })?;
                    if let Some(content) = &binding.content {
                        content.validate_subset(&chat.content).map_err(|_| {
                            error(
                                ProviderCode::UnsupportedCapability,
                                &format!("{base}.content"),
                                id,
                                &binding.adapter,
                            )
                        })?;
                        chat.content = content.clone();
                    }
                    restrict_parameters(&mut chat.parameters, binding.parameters.as_ref())
                        .map_err(|_| {
                            error(
                                ProviderCode::UnsupportedCapability,
                                &format!("{base}.parameters"),
                                id,
                                &binding.adapter,
                            )
                        })?;
                    registry.register_chat(id, chat).map_err(|_| {
                        error(ProviderCode::DuplicateIdentity, &base, id, &binding.adapter)
                    })?;
                } else {
                    let mut media = adapter.media.clone().ok_or_else(|| {
                        error(
                            ProviderCode::UnsupportedOperation,
                            &base,
                            id,
                            &binding.adapter,
                        )
                    })?;
                    if binding.content.is_some() || binding.counting.is_some() {
                        return Err(error(
                            ProviderCode::UnsupportedCapability,
                            &base,
                            id,
                            &binding.adapter,
                        ));
                    }
                    restrict_parameters(&mut media.parameters, binding.parameters.as_ref())
                        .map_err(|_| {
                            error(
                                ProviderCode::UnsupportedCapability,
                                &format!("{base}.parameters"),
                                id,
                                &binding.adapter,
                            )
                        })?;
                    registry.register_media(id, media).map_err(|_| {
                        error(ProviderCode::DuplicateIdentity, &base, id, &binding.adapter)
                    })?;
                }
                adapters.insert((id.clone(), kind.into()), adapter.clone());
            }
        }
        for (id, file) in &configuration.backends {
            if id.is_empty() || id.trim() != id {
                return Err(ProviderConfigurationError::new(
                    ProviderCode::DuplicateIdentity,
                    "backends",
                ));
            }
            let path = format!("backends.{id}");
            legacy_fields(file.endpoint.as_ref(), file.auth.as_ref())
                .map_err(|code| error(code, &path, &file.provider, ""))?;
            let adapter = adapters
                .get(&(file.provider.clone(), "chat".into()))
                .ok_or_else(|| error(ProviderCode::UnknownProvider, &path, &file.provider, ""))?;
            let mut backend = crate::merge_backend(id, file.clone()).map_err(|_| {
                error(
                    ProviderCode::UnsupportedCapability,
                    &path,
                    &file.provider,
                    &adapter.id,
                )
            })?;
            backend.enabled = true;
            if !adapter.chat_streaming.contains(&backend.chat.stream)
                || !adapter
                    .context_overflow
                    .contains(&backend.chat.context_overflow)
                || backend.context_window_tokens == 0
            {
                return Err(error(
                    ProviderCode::UnsupportedCapability,
                    &path,
                    &file.provider,
                    &adapter.id,
                ));
            }
            backend
                .capabilities
                .validate_declared_chat_parameters(&backend.chat)
                .map_err(|_| {
                    error(
                        ProviderCode::UnsupportedCapability,
                        &format!("{path}.properties"),
                        &file.provider,
                        &adapter.id,
                    )
                })?;
            registry.validate_chat_backend(&backend).map_err(|_| {
                error(
                    ProviderCode::UnsupportedCapability,
                    &path,
                    &file.provider,
                    &adapter.id,
                )
            })?;
            options(adapter, &backend.options).map_err(|e| {
                error(
                    e.code,
                    &format!("{path}.options"),
                    &file.provider,
                    &adapter.id,
                )
            })?;
        }
        for (id, file) in &configuration.media_backends {
            if id.is_empty() || id.trim() != id {
                return Err(ProviderConfigurationError::new(
                    ProviderCode::DuplicateIdentity,
                    "media_backends",
                ));
            }
            let path = format!("mediaBackends.{id}");
            legacy_fields(file.endpoint.as_ref(), file.auth.as_ref())
                .map_err(|code| error(code, &path, &file.provider, ""))?;
            let adapter = adapters
                .get(&(file.provider.clone(), "media".into()))
                .ok_or_else(|| error(ProviderCode::UnknownProvider, &path, &file.provider, ""))?;
            let mut backend = crate::merge_media_backend(id, file.clone());
            backend.enabled = true;
            let media = registry
                .media_provider(&file.provider)
                .expect("validated binding");
            if backend
                .capabilities
                .aspect_ratio_options
                .iter()
                .any(|option| !(media.accepts_aspect_ratio)(&backend.model, &option.value))
                || backend
                    .capabilities
                    .size_options
                    .iter()
                    .any(|option| !(media.accepts_size)(&backend.model, &option.value))
            {
                return Err(error(
                    ProviderCode::UnsupportedCapability,
                    &format!("{path}.capabilities"),
                    &file.provider,
                    &adapter.id,
                ));
            }
            registry.validate_media_backend(&backend).map_err(|_| {
                error(
                    ProviderCode::UnsupportedCapability,
                    &path,
                    &file.provider,
                    &adapter.id,
                )
            })?;
            options(adapter, &backend.options).map_err(|e| {
                error(
                    e.code,
                    &format!("{path}.options"),
                    &file.provider,
                    &adapter.id,
                )
            })?;
        }
        Ok(ValidatedProviderConfiguration {
            configuration,
            adapters,
        })
    }
}
fn legacy_fields(
    endpoint: Option<&String>,
    auth: Option<&crate::EngineAuthConfigFile>,
) -> Result<(), ProviderCode> {
    if endpoint.is_some() || auth.is_some() {
        Err(ProviderCode::UpgradeRequired)
    } else {
        Ok(())
    }
}
fn restrict_parameters(
    parameters: &mut Vec<String>,
    restriction: Option<&Vec<String>>,
) -> Result<(), ()> {
    if let Some(restriction) = restriction {
        let unique = restriction
            .iter()
            .collect::<std::collections::BTreeSet<_>>();
        if unique.len() != restriction.len() || restriction.iter().any(|p| !parameters.contains(p))
        {
            return Err(());
        }
        *parameters = restriction.clone();
    }
    Ok(())
}

/// Prepared adapters and the safe configuration snapshot accepted with a run.
pub struct ConfiguredProviders {
    chat: BTreeMap<String, Arc<dyn EngineAdapter>>,
    media: BTreeMap<String, Arc<dyn MediaGenerationAdapter>>,
    snapshot: serde_json::Value,
    revision: String,
}
impl ConfiguredProviders {
    pub fn chat(&self, backend: &str) -> Option<&Arc<dyn EngineAdapter>> {
        self.chat.get(backend)
    }
    pub fn media(&self, backend: &str) -> Option<&Arc<dyn MediaGenerationAdapter>> {
        self.media.get(backend)
    }
    pub fn snapshot(&self) -> &serde_json::Value {
        &self.snapshot
    }
    pub fn revision(&self) -> &str {
        &self.revision
    }
    pub fn verify_revision(&self, revision: &str) -> Result<(), ProviderConfigurationError> {
        if revision == self.revision {
            Ok(())
        } else {
            Err(ProviderConfigurationError::new(
                ProviderCode::ConfigurationMismatch,
                "revision",
            ))
        }
    }
}
impl ValidatedProviderConfiguration {
    pub fn configuration(&self) -> &ProviderConfiguration {
        &self.configuration
    }
    pub fn bind(
        &self,
        credentials: &dyn CredentialResolver,
        logger: Option<Arc<dyn EngineAdapterLogger>>,
    ) -> Result<ConfiguredProviders, ProviderConfigurationError> {
        let mut bindings = BTreeMap::<(String, String), (ProviderBinding, EngineAuthConfig)>::new();
        let mut resolve = |provider: &str,
                           kind: &str|
         -> Result<
            (ProviderBinding, EngineAuthConfig),
            ProviderConfigurationError,
        > {
            let key = (provider.to_owned(), kind.to_owned());
            if let Some(value) = bindings.get(&key) {
                return Ok(value.clone());
            }
            let file = &self.configuration.providers[provider];
            let file = if kind == "chat" {
                file.chat.as_ref()
            } else {
                file.media.as_ref()
            }
            .expect("validated binding");
            let adapter = &self.adapters[&key];
            let credential = file
                .auth
                .credential()
                .map(|r| credentials.resolve(r))
                .transpose()
                .map_err(|e| {
                    error(
                        e.code,
                        &format!("providers.{provider}.{kind}.auth"),
                        provider,
                        &file.adapter,
                    )
                })?;
            if credential
                .as_ref()
                .is_some_and(|c| c.value.expose().trim().is_empty() || c.scope_revision.is_empty())
            {
                return Err(error(
                    ProviderCode::MissingCredential,
                    "auth",
                    provider,
                    &file.adapter,
                ));
            }
            let mut binding = ProviderBinding {
                provider: provider.into(),
                adapter: file.adapter.clone(),
                contract_version: adapter.contract_version,
                chat_streaming: adapter.chat_streaming.clone(),
                context_overflow: adapter.context_overflow.clone(),
                revision: String::new(),
                credential_scope_revision: credential
                    .as_ref()
                    .map(|c| c.scope_revision.clone())
                    .unwrap_or_default(),
                connection: file.connection(),
            };
            binding
                .connection
                .validate_credential(credential.as_ref().map(|c| &c.value))
                .map_err(|e| {
                    error(
                        e.code,
                        &format!("providers.{provider}.{kind}.auth"),
                        provider,
                        &file.adapter,
                    )
                })?;
            binding.seal();
            let auth = EngineAuthConfig {
                auth_type: "resolved".into(),
                api_key_env: String::new(),
                header: None,
                version_header: None,
                version: None,
                has_api_key: credential.is_some(),
                api_key: credential.map(|c| c.value),
            };
            bindings.insert(key, (binding.clone(), auth.clone()));
            Ok((binding, auth))
        };
        let mut chat = BTreeMap::new();
        let mut media = BTreeMap::new();
        for (id, file) in &self.configuration.backends {
            if !file.enabled {
                continue;
            }
            let adapter = &self.adapters[&(file.provider.clone(), "chat".into())];
            let (binding, auth) = resolve(&file.provider, "chat")?;
            let mut backend = crate::merge_backend(id, file.clone()).expect("validated backend");
            backend.endpoint = binding.connection.base_url.clone();
            backend.auth = auth;
            backend.binding = Some(binding);
            let mut registered = adapter.chat.clone().expect("chat adapter");
            let file_binding = self.configuration.providers[&file.provider]
                .chat
                .as_ref()
                .unwrap();
            if let Some(content) = &file_binding.content {
                registered.content = content.clone();
            }
            restrict_parameters(&mut registered.parameters, file_binding.parameters.as_ref())
                .expect("validated parameters");
            let mut registry = ProviderRegistry::default();
            registry
                .register_chat(&file.provider, registered)
                .expect("unique provider");
            let logger = logger.clone().map(|logger| bound::logger(&backend, logger));
            let inner = registry.create_chat(backend.clone(), logger).map_err(|_| {
                error(
                    ProviderCode::InvalidOptions,
                    &format!("backends.{id}"),
                    &file.provider,
                    &adapter.id,
                )
            })?;
            chat.insert(
                id.clone(),
                bound::chat(
                    backend,
                    inner,
                    file_binding.counting.unwrap_or(adapter.counting),
                ),
            );
        }
        for (id, file) in &self.configuration.media_backends {
            if !file.enabled {
                continue;
            }
            let adapter = &self.adapters[&(file.provider.clone(), "media".into())];
            let (binding, auth) = resolve(&file.provider, "media")?;
            let mut backend = crate::merge_media_backend(id, file.clone());
            backend.endpoint = binding.connection.base_url.clone();
            backend.auth = auth;
            backend.binding = Some(binding);
            let mut registered = adapter.media.clone().unwrap();
            restrict_parameters(
                &mut registered.parameters,
                self.configuration.providers[&file.provider]
                    .media
                    .as_ref()
                    .unwrap()
                    .parameters
                    .as_ref(),
            )
            .expect("validated parameters");
            let mut registry = ProviderRegistry::default();
            registry
                .register_media(&file.provider, registered.clone())
                .expect("unique provider");
            let inner = registry.create_media(backend.clone()).map_err(|_| {
                error(
                    ProviderCode::InvalidOptions,
                    &format!("mediaBackends.{id}"),
                    &file.provider,
                    &adapter.id,
                )
            })?;
            media.insert(id.clone(), bound::media(backend, inner, registered));
        }
        let snapshot = serde_json::json!({"version":1,"configuration":self.configuration,"bindings":bindings.values().map(|(b,_)| b).collect::<Vec<_>>()});
        let revision = configuration_digest(&snapshot);
        Ok(ConfiguredProviders {
            chat,
            media,
            snapshot,
            revision,
        })
    }
}
mod bound;

/// Parse declarations without exposing credential-like input in diagnostics.
pub fn parse_provider_configuration(
    source: &str,
) -> Result<ProviderConfiguration, ProviderConfigurationError> {
    serde_yaml::from_str(source).map_err(|error| {
        let code = if error.to_string().contains("duplicate or blank identity") {
            ProviderCode::DuplicateIdentity
        } else {
            ProviderCode::InvalidOptions
        };
        ProviderConfigurationError::new(code, "configuration")
    })
}

/// An explicit assignment for a legacy backend. Keys are `chat/<backend>` or `media/<backend>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyProviderAssignment {
    pub provider: String,
    pub binding: ProtocolBindingFile,
}

/// Convert authored backends using host-supplied service assignments and secret references.
/// The original values and accepted-run records remain owned by the caller.
pub fn upgrade_legacy_providers(
    backends: &BTreeMap<String, EngineBackendConfigFile>,
    media_backends: &BTreeMap<String, MediaBackendConfigFile>,
    assignments: &BTreeMap<String, LegacyProviderAssignment>,
) -> Result<ProviderConfiguration, ProviderConfigurationError> {
    let mut result = ProviderConfiguration {
        version: PROVIDER_CONFIGURATION_VERSION,
        providers: BTreeMap::new(),
        backends: backends.clone(),
        media_backends: media_backends.clone(),
    };
    let mut used = std::collections::BTreeSet::new();
    for (kind, ids) in [
        ("chat", backends.keys().collect::<Vec<_>>()),
        ("media", media_backends.keys().collect::<Vec<_>>()),
    ] {
        for id in ids {
            let key = format!("{kind}/{id}");
            let assignment = assignments.get(&key).ok_or_else(|| {
                ProviderConfigurationError::new(ProviderCode::UpgradeRequired, &key)
            })?;
            if assignment.provider.is_empty() || assignment.provider.trim() != assignment.provider {
                return Err(ProviderConfigurationError::new(
                    ProviderCode::DuplicateIdentity,
                    &key,
                ));
            }
            let provider = result
                .providers
                .entry(assignment.provider.clone())
                .or_default();
            let slot = if kind == "chat" {
                &mut provider.chat
            } else {
                &mut provider.media
            };
            if let Some(existing) = slot {
                if serde_json::to_value(existing).unwrap()
                    != serde_json::to_value(&assignment.binding).unwrap()
                {
                    return Err(ProviderConfigurationError::new(
                        ProviderCode::ConfigurationMismatch,
                        &key,
                    ));
                }
            } else {
                *slot = Some(assignment.binding.clone());
            }
            if kind == "chat" {
                let backend = result.backends.get_mut(id).unwrap();
                backend.provider = assignment.provider.clone();
                backend.endpoint = None;
                backend.auth = None;
            } else {
                let backend = result.media_backends.get_mut(id).unwrap();
                backend.provider = assignment.provider.clone();
                backend.endpoint = None;
                backend.auth = None;
            }
            used.insert(key);
        }
    }
    if assignments.keys().any(|key| !used.contains(key)) {
        return Err(ProviderConfigurationError::new(
            ProviderCode::UnknownProvider,
            "assignments",
        ));
    }
    Ok(result)
}
