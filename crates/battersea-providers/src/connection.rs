//! HTTP composition for an explicitly configured service binding.
#![cfg_attr(
    not(any(
        feature = "openai",
        feature = "anthropic",
        feature = "google",
        feature = "runway",
        feature = "replicate"
    )),
    allow(dead_code)
)]
use battersea_model::{
    Authentication, EngineAdapterRequestError, EngineAuthConfig, ErrorKind, ProviderBinding,
};
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    RequestBuilder,
};

pub(crate) fn headers(
    binding: &ProviderBinding,
    auth: &EngineAuthConfig,
) -> Result<HeaderMap, EngineAdapterRequestError> {
    let invalid = || {
        EngineAdapterRequestError::new(
            &binding.provider,
            "Invalid authentication declaration or credential.",
            ErrorKind::InvalidRequest,
        )
    };
    let mut headers = HeaderMap::new();
    for (key, value) in &binding.connection.headers {
        headers.insert(
            HeaderName::from_bytes(key.as_bytes()).map_err(|_| invalid())?,
            HeaderValue::from_str(value).map_err(|_| invalid())?,
        );
    }
    let header = match &binding.connection.auth {
        Authentication::None | Authentication::Query { .. } => None,
        Authentication::Bearer { .. } => Some((
            "authorization",
            format!("Bearer {}", auth.api_key.as_deref().ok_or_else(invalid)?),
        )),
        Authentication::Header { header, .. } => Some((
            header.as_str(),
            auth.api_key.as_deref().ok_or_else(invalid)?.to_owned(),
        )),
    };
    if let Some((name, value)) = header {
        let mut value = HeaderValue::from_str(&value).map_err(|_| invalid())?;
        value.set_sensitive(true);
        headers.insert(
            HeaderName::from_bytes(name.as_bytes()).map_err(|_| invalid())?,
            value,
        );
    }
    Ok(headers)
}
pub(crate) fn authenticate(
    request: RequestBuilder,
    binding: Option<&ProviderBinding>,
    auth: &EngineAuthConfig,
) -> Result<RequestBuilder, EngineAdapterRequestError> {
    let Some(binding) = binding else {
        return Ok(request);
    };
    let request = request.headers(headers(binding, auth)?);
    Ok(match &binding.connection.auth {
        Authentication::Query { parameter, .. } => request.query(&[(
            parameter,
            auth.api_key.as_deref().ok_or_else(|| {
                EngineAdapterRequestError::new(
                    &binding.provider,
                    "Missing credential.",
                    ErrorKind::InvalidRequest,
                )
            })?,
        )]),
        _ => request,
    })
}
#[cfg(any(feature = "openai", feature = "anthropic", feature = "runway"))]
pub(crate) fn base(binding: Option<&ProviderBinding>, legacy: impl FnOnce() -> String) -> String {
    binding
        .map(|b| b.connection.base_url.trim_end_matches('/').to_owned())
        .unwrap_or_else(legacy)
}
#[cfg(any(feature = "openai", feature = "google"))]
pub(crate) fn segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}
