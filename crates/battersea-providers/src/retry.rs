//! Retries of immutable HTTP requests, below the conversation/tool-effect boundary.
use crate::{
    EngineAdapterLogger, EngineAdapterRequestError, EngineAdapterRetryLog, EngineBackendConfig,
};
use battersea_model::adapter::error::{DispatchState, ErrorKind};
use reqwest::{RequestBuilder, Response};
use std::{sync::Arc, time::Duration};

pub(crate) fn validate(backend: &EngineBackendConfig) -> Result<(), EngineAdapterRequestError> {
    if backend.options.max_retries.unwrap_or(0) > 0 {
        match (backend.options.retry_initial_delay_ms, backend.options.retry_max_delay_ms) {
            (Some(initial), Some(maximum)) if initial > 0 && maximum >= initial => {},
            _ => return Err(EngineAdapterRequestError::new(&backend.provider,
                "Retries require positive retryInitialDelayMs and retryMaxDelayMs >= retryInitialDelayMs.", ErrorKind::InvalidRequest)),
        }
    }
    Ok(())
}

fn delay_ms(backend: &EngineBackendConfig, retry: u32, after: Option<u64>) -> Option<u64> {
    let initial = backend.options.retry_initial_delay_ms?;
    let maximum = backend.options.retry_max_delay_ms?;
    if after.is_some_and(|ms| ms > maximum) {
        return None;
    }
    Some(
        initial
            .saturating_mul(2u64.checked_pow(retry).unwrap_or(u64::MAX))
            .min(maximum)
            .max(after.unwrap_or(0)),
    )
}

fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(seconds.saturating_mul(1000));
    }
    chrono::DateTime::parse_from_rfc2822(value)
        .ok()
        .map(|date| {
            date.signed_duration_since(chrono::Utc::now())
                .num_milliseconds()
                .max(0) as u64
        })
}

pub(crate) async fn send(
    request: RequestBuilder,
    backend: &EngineBackendConfig,
    logger: Option<&Arc<dyn EngineAdapterLogger>>,
) -> Result<Response, EngineAdapterRequestError> {
    validate(backend)?;
    // Building before sending separates malformed requests from ambiguous transport failures.
    let (client, prepared) = request.build_split();
    let prepared = prepared.map_err(|e| {
        EngineAdapterRequestError::new(
            &backend.provider,
            e.without_url().to_string(),
            ErrorKind::InvalidRequest,
        )
    })?;
    let retrieval = prepared.method() == reqwest::Method::GET;
    let retries = backend.options.max_retries.unwrap_or(0);
    for retry in 0..=retries {
        let attempt = prepared.try_clone().ok_or_else(|| {
            EngineAdapterRequestError::new(
                &backend.provider,
                "Request body cannot be replayed.",
                ErrorKind::InvalidRequest,
            )
        })?;
        let mut failure = match client.execute(attempt).await {
            Ok(response) if response.status().is_success() => return Ok(response),
            Ok(response) => {
                let status = response.status().as_u16();
                let classification = match status {
                    401 => ErrorKind::Authentication,
                    403 => ErrorKind::Permission,
                    429 => ErrorKind::RateLimit,
                    500..=599 => ErrorKind::Server,
                    _ => ErrorKind::Request,
                };
                let mut error = EngineAdapterRequestError::new(
                    &backend.provider,
                    format!("Provider HTTP request failed with status {status}."),
                    classification,
                )
                .with_status_code(status)
                .with_dispatch(if status < 500 {
                    DispatchState::Rejected
                } else {
                    DispatchState::Unknown
                });
                error.request_id = ["request-id", "x-request-id"].iter().find_map(|name| {
                    response
                        .headers()
                        .get(*name)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_owned)
                });
                error.retry_after_ms = retry_after(response.headers());
                if response
                    .headers()
                    .contains_key(reqwest::header::RETRY_AFTER)
                    && error.retry_after_ms.is_none()
                {
                    // An unreadable provider delay cannot authorise an earlier retry.
                    error.retry_after_ms = Some(u64::MAX);
                }
                // A rejection's body may contain credentials or prompt material. Status and identity
                // are sufficient for policy; bounded provider-specific diagnostics remain separate.
                error
            }
            Err(error) => {
                let not_sent = error.is_connect() || error.is_builder();
                let kind = if error.is_builder() {
                    ErrorKind::InvalidRequest
                } else if error.is_timeout() {
                    ErrorKind::Timeout
                } else {
                    ErrorKind::Transport
                };
                EngineAdapterRequestError::new(
                    &backend.provider,
                    error.without_url().to_string(),
                    kind,
                )
                .with_dispatch(if not_sent {
                    DispatchState::NotSent
                } else {
                    DispatchState::Unknown
                })
            }
        };
        failure.attempts = retry.saturating_add(1);
        let safe = (failure.dispatch == DispatchState::NotSent
            && failure.classification != ErrorKind::InvalidRequest)
            || failure.status_code == Some(429)
            || (retrieval
                && matches!(
                    failure.classification,
                    ErrorKind::Transport | ErrorKind::Timeout | ErrorKind::Server
                ));
        if retry == retries || !safe {
            return Err(failure);
        }
        let Some(delay_ms) = delay_ms(backend, retry, failure.retry_after_ms) else {
            return Err(failure);
        };
        if let Some(logger) = logger {
            logger
                .on_retry(EngineAdapterRetryLog {
                    provider: backend.provider.clone(),
                    backend: backend.id.clone(),
                    attempt: retry.saturating_add(2),
                    delay_ms,
                    dispatch: failure.dispatch,
                    status_code: failure.status_code,
                    request_id: failure.request_id.clone(),
                    resume: retrieval,
                })
                .await;
        }
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
    unreachable!("finite attempt loop returns a response or failure")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_after_is_a_lower_bound_including_http_dates() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("retry-after", "3".parse().unwrap());
        assert_eq!(retry_after(&headers), Some(3000));
        let future = chrono::Utc::now() + chrono::Duration::seconds(10);
        headers.insert("retry-after", future.to_rfc2822().parse().unwrap());
        assert!(retry_after(&headers).is_some_and(|ms| (8000..=10000).contains(&ms)));
    }
}
