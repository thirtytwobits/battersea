//! Protocol implementations and immutable configured service identities.
mod redaction;

use crate::{ChatProvider, EngineAdapterRequestError, ErrorKind, MediaProvider};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt, sync::Arc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProviderCode {
    UnknownAdapter,
    DuplicateIdentity,
    UnsupportedVersion,
    UnknownProvider,
    UnsupportedOperation,
    UnsupportedCapability,
    InvalidOptions,
    InvalidEndpoint,
    InvalidAuthentication,
    MissingCredential,
    ConfigurationMismatch,
    IncompatibleContinuation,
    UpgradeRequired,
}

/// A configuration refusal. Paths and identities identify the failing declaration; values are omitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{code:?} at {path}")]
pub struct ProviderConfigurationError {
    pub code: ProviderCode,
    pub path: String,
    pub provider: Option<String>,
    pub adapter: Option<String>,
}
impl ProviderConfigurationError {
    pub fn new(code: ProviderCode, path: impl Into<String>) -> Self {
        Self {
            code,
            path: path.into(),
            provider: None,
            adapter: None,
        }
    }
    pub fn at_binding(mut self, provider: &str, adapter: &str) -> Self {
        self.provider = Some(provider.into());
        self.adapter = Some(adapter.into());
        self
    }
}

/// Secret bytes are deliberately excluded from serialisable configuration.
#[derive(Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct SecretString(String);
impl SecretString {
    pub fn expose(&self) -> &str {
        &self.0
    }
    /// Mask raw, URI/form-encoded and JSON-escaped representations in diagnostics.
    pub fn redact(&self, text: &str) -> String {
        redaction::redact(&self.0, text)
    }
}
impl From<String> for SecretString {
    fn from(value: String) -> Self {
        Self(value)
    }
}
impl From<&str> for SecretString {
    fn from(value: &str) -> Self {
        Self(value.into())
    }
}
impl std::ops::Deref for SecretString {
    type Target = str;
    fn deref(&self) -> &str {
        self.expose()
    }
}
impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialReference {
    Env(String),
    Host(String),
}
impl Serialize for CredentialReference {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            Self::Env(name) => map.serialize_entry("env", name)?,
            Self::Host(name) => map.serialize_entry("host", name)?,
        }
        map.end()
    }
}
impl<'de> Deserialize<'de> for CredentialReference {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged, deny_unknown_fields)]
        enum Reference {
            Env { env: String },
            Host { host: String },
        }
        Ok(match Reference::deserialize(deserializer)? {
            Reference::Env { env } => Self::Env(env),
            Reference::Host { host } => Self::Host(host),
        })
    }
}

/// Hosts associate a scope revision with credentials belonging to the same remote principal.
#[derive(Debug, Clone)]
pub struct ResolvedCredential {
    pub value: SecretString,
    pub scope_revision: String,
}
pub trait CredentialResolver: Send + Sync {
    fn resolve(
        &self,
        reference: &CredentialReference,
    ) -> Result<ResolvedCredential, ProviderConfigurationError>;
}
/// The host supplies a scope revision and changes it when the remote account changes.
pub struct EnvironmentCredentials {
    pub scope_revision: String,
}
impl CredentialResolver for EnvironmentCredentials {
    fn resolve(
        &self,
        reference: &CredentialReference,
    ) -> Result<ResolvedCredential, ProviderConfigurationError> {
        let CredentialReference::Env(name) = reference else {
            return Err(ProviderConfigurationError::new(
                ProviderCode::MissingCredential,
                "auth.credential",
            ));
        };
        let value = std::env::var(name)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| {
                ProviderConfigurationError::new(ProviderCode::MissingCredential, "auth.credential")
            })?;
        if self.scope_revision.is_empty() {
            return Err(ProviderConfigurationError::new(
                ProviderCode::MissingCredential,
                "auth.scopeRevision",
            ));
        }
        Ok(ResolvedCredential {
            value: value.into(),
            scope_revision: self.scope_revision.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Authentication {
    None,
    Bearer {
        credential: CredentialReference,
    },
    Header {
        header: String,
        credential: CredentialReference,
    },
    Query {
        parameter: String,
        credential: CredentialReference,
    },
}
impl Authentication {
    pub fn credential(&self) -> Option<&CredentialReference> {
        match self {
            Self::None => None,
            Self::Bearer { credential }
            | Self::Header { credential, .. }
            | Self::Query { credential, .. } => Some(credential),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderConnection {
    pub base_url: String,
    pub auth: Authentication,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub delivery_origins: Vec<String>,
}
impl ProviderConnection {
    pub fn normalise(&self) -> Result<Self, ProviderConfigurationError> {
        let bad_url = || ProviderConfigurationError::new(ProviderCode::InvalidEndpoint, "baseUrl");
        // Empty connections are useful for local adapters; the adapter descriptor admits them.
        let mut result = self.clone();
        if !self.base_url.is_empty() {
            let mut url = url::Url::parse(&self.base_url).map_err(|_| bad_url())?;
            if !matches!(url.scheme(), "http" | "https")
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(bad_url());
            }
            let path = format!("{}/", url.path().trim_end_matches('/'));
            url.set_path(&path);
            result.base_url = url.to_string();
        }
        let bad_auth =
            || ProviderConfigurationError::new(ProviderCode::InvalidAuthentication, "auth");
        let reserved = match &self.auth {
            Authentication::Bearer { .. } => Some("authorization".to_owned()),
            Authentication::Header { header, .. } => {
                let name = http::header::HeaderName::from_bytes(header.as_bytes())
                    .map_err(|_| bad_auth())?;
                if matches!(
                    name.as_str(),
                    "host" | "content-length" | "content-type" | "transfer-encoding" | "connection"
                ) {
                    return Err(bad_auth());
                }
                if let Authentication::Header { header, .. } = &mut result.auth {
                    *header = name.to_string();
                }
                Some(name.to_string())
            }
            Authentication::Query { parameter, .. } => {
                if parameter.is_empty()
                    || !parameter
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                {
                    return Err(bad_auth());
                }
                None
            }
            Authentication::None => None,
        };
        if let Some(reference) = self.auth.credential() {
            let (CredentialReference::Env(name) | CredentialReference::Host(name)) = reference;
            if name.trim().is_empty() {
                return Err(bad_auth());
            }
        }
        result.headers.clear();
        for (key, value) in &self.headers {
            let name =
                http::header::HeaderName::from_bytes(key.as_bytes()).map_err(|_| bad_auth())?;
            http::header::HeaderValue::from_str(value).map_err(|_| bad_auth())?;
            if matches!(
                name.as_str(),
                "authorization"
                    | "x-api-key"
                    | "x-goog-api-key"
                    | "api-key"
                    | "proxy-authorization"
                    | "cookie"
                    | "host"
                    | "content-length"
                    | "transfer-encoding"
                    | "connection"
            ) || reserved.as_deref() == Some(name.as_str())
                || result
                    .headers
                    .insert(name.to_string(), value.clone())
                    .is_some()
            {
                return Err(bad_auth());
            }
        }
        for origin in &mut result.delivery_origins {
            let parsed = url::Url::parse(origin).map_err(|_| bad_url())?;
            if parsed.scheme() != "https"
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.query().is_some()
                || parsed.fragment().is_some()
                || parsed.path() != "/"
            {
                return Err(bad_url());
            }
            *origin = parsed.origin().ascii_serialization();
        }
        result.delivery_origins.sort();
        result.delivery_origins.dedup();
        Ok(result)
    }
    /// Validate resolved authentication bytes before constructing an adapter.
    pub fn validate_credential(
        &self,
        credential: Option<&SecretString>,
    ) -> Result<(), ProviderConfigurationError> {
        let invalid =
            || ProviderConfigurationError::new(ProviderCode::InvalidAuthentication, "auth");
        match &self.auth {
            Authentication::None => {
                if credential.is_some() {
                    return Err(invalid());
                }
            }
            Authentication::Bearer { .. } | Authentication::Header { .. } => {
                let value = credential.ok_or_else(invalid)?;
                http::header::HeaderValue::from_str(value.expose()).map_err(|_| invalid())?;
            }
            Authentication::Query { .. } => {
                credential.ok_or_else(invalid)?;
            }
        }
        Ok(())
    }
    /// Append a protocol-owned relative route, preserving the configured path prefix.
    pub fn route(&self, path: &str) -> Result<url::Url, EngineAdapterRequestError> {
        let invalid = || {
            EngineAdapterRequestError::new(
                "configuration",
                "Invalid relative protocol route.",
                ErrorKind::InvalidRequest,
            )
        };
        if path.starts_with('/')
            || path.contains('\\')
            || path.split('/').any(|p| p == ".." || p == ".")
        {
            return Err(invalid());
        }
        let base = url::Url::parse(&self.base_url).map_err(|_| invalid())?;
        let url = base.join(path).map_err(|_| invalid())?;
        if url.origin() != base.origin()
            || !url.path().starts_with(base.path())
            || url.fragment().is_some()
        {
            return Err(invalid());
        }
        Ok(url)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderContentOrigin {
    pub provider: String,
    pub adapter: String,
    pub binding_revision: String,
}

/// Safe to persist with accepted configuration; credentials remain in the runtime authentication value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderBinding {
    pub provider: String,
    pub adapter: String,
    pub contract_version: u32,
    pub chat_streaming: Vec<bool>,
    pub context_overflow: Vec<crate::engine::EngineContextOverflow>,
    pub revision: String,
    pub credential_scope_revision: String,
    pub connection: ProviderConnection,
}
impl ProviderBinding {
    pub fn origin(&self) -> ProviderContentOrigin {
        ProviderContentOrigin {
            provider: self.provider.clone(),
            adapter: self.adapter.clone(),
            binding_revision: self.revision.clone(),
        }
    }
    pub fn seal(&mut self) {
        self.revision.clear();
        self.revision =
            configuration_digest(&serde_json::to_value(&self).expect("serialisable binding"));
    }
    pub fn error(
        &self,
        mut error: EngineAdapterRequestError,
        secret: Option<&str>,
    ) -> EngineAdapterRequestError {
        error.provider = self.provider.clone().into_boxed_str();
        if let Some(secret) = secret.filter(|s| !s.is_empty()) {
            error.message = SecretString::from(secret)
                .redact(&error.message)
                .into_boxed_str();
            error.request_id = error
                .request_id
                .map(|id| SecretString::from(secret).redact(&id).into_boxed_str());
            if let Some(evidence) = &mut error.tool_call {
                evidence.request_id = evidence
                    .request_id
                    .take()
                    .map(|id| SecretString::from(secret).redact(&id));
            }
        }
        error
    }
}

/// Object keys are sorted recursively so authored map order does not affect the revision.
pub fn configuration_digest(value: &Value) -> String {
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), canonical(v)))
                    .collect::<BTreeMap<_, _>>()
                    .into_iter()
                    .collect(),
            ),
            Value::Array(items) => Value::Array(items.iter().map(canonical).collect()),
            value => value.clone(),
        }
    }
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&canonical(value)).expect("JSON value"))
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TokenCounting {
    Native,
    Estimate,
    Unavailable,
}

/// An implementation ceiling. Services may further restrict the offered operations.
#[derive(Clone)]
pub struct ProtocolAdapter {
    pub id: String,
    pub contract_version: u32,
    pub chat_streaming: Vec<bool>,
    pub context_overflow: Vec<crate::engine::EngineContextOverflow>,
    pub chat: Option<ChatProvider>,
    pub media: Option<MediaProvider>,
    pub counting: TokenCounting,
    pub requires_endpoint: bool,
    pub required_headers: Vec<String>,
    pub options_schema: Value,
}
#[derive(Default)]
pub struct ProtocolRegistry {
    adapters: BTreeMap<String, Arc<ProtocolAdapter>>,
}
impl ProtocolRegistry {
    pub fn register(&mut self, adapter: ProtocolAdapter) -> Result<(), ProviderConfigurationError> {
        if adapter.id.trim().is_empty()
            || adapter.id.trim() != adapter.id
            || self.adapters.contains_key(&adapter.id)
        {
            return Err(ProviderConfigurationError::new(
                ProviderCode::DuplicateIdentity,
                "adapter.id",
            ));
        }
        if adapter.contract_version == 0
            || (adapter.chat.is_none() && adapter.media.is_none())
            || (adapter.chat.is_some()
                && (adapter.chat_streaming.is_empty() || adapter.context_overflow.is_empty()))
        {
            return Err(ProviderConfigurationError::new(
                ProviderCode::UnsupportedOperation,
                "adapter",
            ));
        }
        jsonschema::validator_for(&adapter.options_schema).map_err(|_| {
            ProviderConfigurationError::new(ProviderCode::InvalidOptions, "adapter.optionsSchema")
        })?;
        self.adapters.insert(adapter.id.clone(), Arc::new(adapter));
        Ok(())
    }
    pub fn adapter(&self, id: &str) -> Result<&Arc<ProtocolAdapter>, ProviderConfigurationError> {
        self.adapters
            .get(id)
            .ok_or_else(|| ProviderConfigurationError::new(ProviderCode::UnknownAdapter, "adapter"))
    }
    pub fn adapters(&self) -> impl Iterator<Item = &ProtocolAdapter> {
        self.adapters.values().map(Arc::as_ref)
    }
}

/// Explicitly attach an origin using the host's retained legacy provider assignment.
/// The caller supplies evidence that the legacy content belongs to the accepted binding.
pub fn upgrade_provider_content(
    message: &crate::Message,
    legacy_provider: &str,
    origin: &ProviderContentOrigin,
) -> Result<crate::Message, ProviderConfigurationError> {
    let mut result = message.clone();
    for block in &mut result.content {
        *block = match block.clone() {
            crate::ContentBlock::Native { provider, payload } if provider == legacy_provider => {
                crate::ContentBlock::NativeV2 {
                    origin: origin.clone(),
                    payload,
                }
            }
            crate::ContentBlock::Reasoning {
                provider,
                text,
                signature,
            } if provider == legacy_provider => crate::ContentBlock::ReasoningV2 {
                origin: origin.clone(),
                text,
                signature,
            },
            crate::ContentBlock::Native { .. } | crate::ContentBlock::Reasoning { .. } => {
                return Err(ProviderConfigurationError::new(
                    ProviderCode::IncompatibleContinuation,
                    "content.origin",
                ))
            }
            block => block,
        };
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(base_url: &str) -> ProviderConnection {
        ProviderConnection {
            base_url: base_url.into(),
            auth: Authentication::None,
            headers: BTreeMap::new(),
            delivery_origins: vec![],
        }
    }

    #[test]
    fn binding_errors_redact_nested_tool_request_ids_and_preserve_outcomes() {
        let binding = ProviderBinding {
            provider: "service".into(),
            adapter: "test".into(),
            contract_version: 1,
            chat_streaming: vec![true],
            context_overflow: vec![],
            revision: "binding".into(),
            credential_scope_revision: "account".into(),
            connection: connection("https://service.example/v1/"),
        };
        for request_id in ["special+ /token", "special%2B+%2Ftoken"] {
            let error = EngineAdapterRequestError::transport("remote", "tool failed")
                .with_tool_call(crate::ToolCallEvidence {
                    call_id: "call".into(),
                    tool_name: "lookup".into(),
                    outcome: crate::ToolCallOutcome::Uncertain,
                    request_id: Some(format!("request-{request_id}")),
                });
            let error = binding.error(error, Some("special+ /token"));
            let wire = serde_json::to_value(&error).unwrap();
            assert_eq!(wire["tool_call"]["request_id"], "request-[REDACTED]");
            assert!(!format!("{error:?}").contains(request_id));
            let evidence = error.tool_call.unwrap();
            assert_eq!(evidence.call_id, "call");
            assert_eq!(evidence.tool_name, "lookup");
            assert_eq!(evidence.outcome, crate::ToolCallOutcome::Uncertain);
        }
    }

    #[test]
    fn api_directories_preserve_proxy_prefixes_and_refuse_escaping_routes() {
        let binding = connection("https://service.example/proxy/v2")
            .normalise()
            .unwrap();
        assert_eq!(
            binding.route("models/a%2Fb:countTokens").unwrap().as_str(),
            "https://service.example/proxy/v2/models/a%2Fb:countTokens"
        );
        for path in [
            "/responses",
            "../responses",
            "%2e%2e/responses",
            "https://other.example/responses",
            "//other.example/responses",
            "responses#fragment",
            r"..\responses",
        ] {
            assert!(binding.route(path).is_err(), "{path}");
        }
        for base in [
            "ftp://service.example/",
            "https://user:secret@service.example/",
            "https://service.example/?key=secret",
            "https://service.example/#fragment",
        ] {
            assert!(connection(base).normalise().is_err());
        }
    }

    #[test]
    fn authentication_owns_credential_headers_and_validates_delivery_origins() {
        let mut value = connection("https://service.example/v1/");
        value.auth = Authentication::Header {
            header: "X-Account-Key".into(),
            credential: CredentialReference::Host("account".into()),
        };
        value
            .headers
            .insert("X-Protocol-Version".into(), "v2".into());
        let normal = value.normalise().unwrap();
        assert!(normal.headers.contains_key("x-protocol-version"));
        assert!(
            matches!(normal.auth,Authentication::Header { ref header,.. } if header=="x-account-key")
        );
        value
            .headers
            .insert("x-account-key".into(), "literal-secret".into());
        assert_eq!(
            value.normalise().unwrap_err().code,
            ProviderCode::InvalidAuthentication
        );
        value.headers.clear();
        value.delivery_origins = vec!["https://assets.example/path".into()];
        assert_eq!(
            value.normalise().unwrap_err().code,
            ProviderCode::InvalidEndpoint
        );
        value.delivery_origins = vec!["https://assets.example/".into()];
        assert_eq!(
            value.normalise().unwrap().delivery_origins,
            vec!["https://assets.example"]
        );
        let secret = SecretString::from("special+ /token");
        assert_eq!(format!("{secret:?}"), "[REDACTED]");
        assert_eq!(
            secret.redact("special+ /token special%2B+%2Ftoken"),
            "[REDACTED] [REDACTED]"
        );
    }
}
