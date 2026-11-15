//! Retries of immutable HTTP requests, below the conversation/tool-effect boundary.
use crate::http_payload::BoundedResponse as _;
use crate::{EngineAdapterLogger, EngineAdapterRetryLog, EngineBackendConfig, Failure};
use battersea_model::{
    adapter::error::{DispatchState, ErrorKind},
    Clock,
};
use reqwest::{RequestBuilder, Response};
use std::{sync::Arc, time::Duration};

pub(crate) fn validate(backend: &EngineBackendConfig) -> Result<(), Failure> {
    if backend.options.max_retries.unwrap_or(0) > 0 {
        match (backend.options.retry_initial_delay_ms, backend.options.retry_max_delay_ms) {
            (Some(initial), Some(maximum)) if initial > 0 && maximum >= initial => {},
            _ => return Err(Failure::new(&backend.provider,
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

/// The provider's requested delay. An HTTP date is measured from `clock`'s wall time.
fn retry_after(headers: &reqwest::header::HeaderMap, clock: &dyn Clock) -> Option<u64> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(seconds.saturating_mul(1000));
    }
    let date = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let now = i64::try_from(clock.unix_ms()?).ok()?;
    Some(date.timestamp_millis().saturating_sub(now).max(0) as u64)
}

/// The identifier a provider issued for a request, from its response headers.
fn request_id(headers: &reqwest::header::HeaderMap) -> Option<String> {
    ["request-id", "x-request-id"].iter().find_map(|name| {
        headers
            .get(*name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    })
}

/// The event that announces the identifiers `response` carries. An adapter emits it before
/// any other event of the response, so a later failure of the stream carries them.
pub(crate) fn opened(response: &Response) -> battersea_model::EngineTextStreamEvent {
    battersea_model::EngineTextStreamEvent::ResponseOpened {
        request_id: request_id(response.headers()),
        response_id: None,
    }
}

/// The event that announces the response identifier a provider payload holds at `id`.
pub(crate) fn identified(
    id: Option<&serde_json::Value>,
) -> Option<battersea_model::EngineTextStreamEvent> {
    let id = id?.as_str()?;
    Some(battersea_model::EngineTextStreamEvent::ResponseOpened {
        request_id: None,
        response_id: Some(id.to_owned()),
    })
}

/// Sends `request`, retrying safe failures with backoff timed by `clock`. The caller's operation
/// deadline bounds every attempt and delay.
pub(crate) async fn send(
    request: RequestBuilder,
    backend: &EngineBackendConfig,
    logger: Option<&Arc<dyn EngineAdapterLogger>>,
    clock: &dyn Clock,
) -> Result<Response, Failure> {
    validate(backend)?;
    // Building before sending separates malformed requests from ambiguous transport failures.
    let request =
        crate::connection::authenticate(request, backend.binding.as_ref(), &backend.auth)?;
    let (client, prepared) = request.build_split();
    let prepared = prepared.map_err(|e| {
        Failure::new(
            &backend.provider,
            e.without_url().to_string(),
            ErrorKind::InvalidRequest,
        )
    })?;
    let retrieval = prepared.method() == reqwest::Method::GET;
    let retries = backend.options.max_retries.unwrap_or(0);
    for retry in 0..=retries {
        let attempt = prepared.try_clone().ok_or_else(|| {
            Failure::new(
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
                let request_id = request_id(response.headers());
                let delay_named = response
                    .headers()
                    .contains_key(reqwest::header::RETRY_AFTER);
                let retry_after_ms = retry_after(response.headers(), clock);
                // An unreadable body leaves the status as the whole account of the rejection.
                let body = response
                    .bounded_text()
                    .await
                    .map(crate::http_payload::rejection_body);
                let mut error = Failure::new(
                    &backend.provider,
                    "Provider rejected the HTTP request.",
                    classification,
                )
                .with_status_code(status)
                .with_dispatch(if status < 500 {
                    DispatchState::Rejected
                } else {
                    DispatchState::Unknown
                });
                error.reporter = battersea_model::Reporter::Provider;
                match body {
                    Ok(Some(body)) => error.native = battersea_model::Detail::Present(body),
                    Ok(None) => {}
                    Err(unreadable) => error = error.with_cause(unreadable),
                }
                error.exchange.request_id = request_id;
                error.exchange.retry_after_ms = retry_after_ms;
                if delay_named && error.exchange.retry_after_ms.is_none() {
                    // An unreadable provider delay cannot authorise an earlier retry.
                    error.exchange.retry_after_ms = Some(u64::MAX);
                }
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
                Failure::new(&backend.provider, error.without_url().to_string(), kind)
                    .with_dispatch(if not_sent {
                        DispatchState::NotSent
                    } else {
                        DispatchState::Unknown
                    })
            }
        };
        failure.exchange.attempts = retry.saturating_add(1);
        let safe = (failure.exchange.dispatch == DispatchState::NotSent
            && failure.kind != ErrorKind::InvalidRequest)
            || failure.exchange.status == Some(429)
            || (retrieval
                && matches!(
                    failure.kind,
                    ErrorKind::Transport | ErrorKind::Timeout | ErrorKind::Server
                ));
        if retry == retries || !safe {
            return Err(failure);
        }
        let Some(delay_ms) = delay_ms(backend, retry, failure.exchange.retry_after_ms) else {
            return Err(failure);
        };
        if let Some(logger) = logger {
            logger
                .on_retry(EngineAdapterRetryLog {
                    provider: backend.provider.clone(),
                    backend: backend.id.clone(),
                    attempt: retry.saturating_add(2),
                    delay_ms,
                    dispatch: failure.exchange.dispatch,
                    status_code: failure.exchange.status,
                    request_id: failure.exchange.request_id.as_deref().map(str::to_owned),
                    resume: retrieval,
                })
                .await;
        }
        battersea_model::clock::sleep(clock, Duration::from_millis(delay_ms)).await;
    }
    unreachable!("finite attempt loop returns a response or failure")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// contracts/failures.md: native detail is the body the provider sent, whole.
    #[test]
    fn a_rejection_body_is_kept_whole_as_json_or_as_text() {
        let json = r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"},"request_id":"req_1"}"#;
        assert_eq!(
            crate::http_payload::rejection_body(json.into()),
            Some(serde_json::from_str(json).unwrap())
        );
        assert_eq!(
            crate::http_payload::rejection_body("upstream unavailable".into()),
            Some(serde_json::Value::String("upstream unavailable".into()))
        );
        assert_eq!(crate::http_payload::rejection_body(String::new()), None);
    }
    #[test]
    fn retry_after_is_a_lower_bound_including_http_dates() {
        // Sun, 06 Nov 1994 08:49:37 GMT, the RFC 9110 example date.
        let clock = battersea_model::ManualClock::new(784_111_777_000);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("retry-after", "3".parse().unwrap());
        assert_eq!(retry_after(&headers, &clock), Some(3000));
        headers.insert(
            "retry-after",
            "Sun, 06 Nov 1994 08:49:47 GMT".parse().unwrap(),
        );
        assert_eq!(retry_after(&headers, &clock), Some(10_000));
        clock.advance(Duration::from_secs(4));
        assert_eq!(retry_after(&headers, &clock), Some(6_000));
        clock.advance(Duration::from_secs(20));
        assert_eq!(retry_after(&headers, &clock), Some(0));
    }
}
