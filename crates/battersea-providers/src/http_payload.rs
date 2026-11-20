//! Bound raw HTTP bytes before JSON or SSE decoding retains them.
use crate::Failure;
use battersea_model::adapter::payload::PROVIDER_PAYLOAD_BYTES;
use futures_util::{Stream, StreamExt};
use serde::de::DeserializeOwned;

pub(crate) fn bounded_stream(
    response: reqwest::Response,
) -> impl Stream<Item = Result<impl AsRef<[u8]>, Failure>> + Send {
    // reqwest prints the request URL, which can hold a query credential.
    limit_stream(
        response
            .bytes_stream()
            .map(|chunk| chunk.map_err(reqwest::Error::without_url)),
        PROVIDER_PAYLOAD_BYTES,
    )
}

fn limit_stream<T: AsRef<[u8]>, E: std::error::Error + 'static>(
    stream: impl Stream<Item = Result<T, E>>,
    limit: usize,
) -> impl Stream<Item = Result<T, Failure>> {
    stream.scan(Some(limit), |remaining, chunk| {
        let value = match (remaining.as_mut(), chunk) {
            (None, _) => None,
            (Some(left), Ok(chunk)) if chunk.as_ref().len() <= *left => {
                *left -= chunk.as_ref().len();
                Some(Ok(chunk))
            }
            (Some(_), Ok(_)) => {
                *remaining = None;
                Some(Err(Failure::invalid_response(
                    "http",
                    "Provider response exceeds its encoded-byte limit.",
                )))
            }
            (Some(_), Err(error)) => {
                *remaining = None;
                Some(Err(Failure::transport(
                    "http",
                    "Provider response stream failed.",
                )
                .caused_by(&error)))
            }
        };
        std::future::ready(value)
    })
}

/// An HTTP client that could not be built from its backend's configuration.
pub(crate) fn client_failure(provider: &str, error: reqwest::Error) -> Failure {
    Failure::transport(provider, "HTTP client could not be built.").caused_by(&error.without_url())
}

/// A request that failed before its response arrived. reqwest prints the request URL, which
/// can hold a query credential.
pub(crate) fn send_failure(provider: &str, error: reqwest::Error) -> Failure {
    Failure::transport(provider, "HTTP request failed.").caused_by(&error.without_url())
}

/// A failed read of a provider event stream. A failure of the response body beneath the event
/// stream is the cause, whole; the event stream's own error is a foreign cause.
pub(crate) fn event_stream_failure(
    failure: Failure,
    error: eventsource_stream::EventStreamError<Failure>,
) -> Failure {
    match error {
        eventsource_stream::EventStreamError::Transport(lower) => failure.with_cause(lower),
        other => failure.caused_by(&other),
    }
}

/// Carries `payload`, the provider payload Battersea was handling when it raised `failure`, as
/// native detail on a failure that holds none.
pub(crate) fn about(failure: Failure, payload: &serde_json::Value) -> Failure {
    if failure.native.is_absent() && !payload.is_null() {
        failure.with_native(battersea_model::Reporter::Provider, payload.clone())
    } else {
        failure
    }
}

/// A rejection's body as the provider sent it: its JSON value, else its text.
pub(crate) fn rejection_body(body: String) -> Option<serde_json::Value> {
    if body.is_empty() {
        return None;
    }
    Some(serde_json::from_str(&body).unwrap_or(serde_json::Value::String(body)))
}

#[allow(clippy::double_must_use)] // async_trait marks its boxed Future must_use.
#[async_trait::async_trait]
pub(crate) trait BoundedResponse {
    async fn bounded_text(self) -> Result<String, Failure>;
    async fn bounded_json<T: DeserializeOwned + Send>(self) -> Result<T, Failure>;
}
#[async_trait::async_trait]
impl BoundedResponse for reqwest::Response {
    async fn bounded_text(self) -> Result<String, Failure> {
        let stream = bounded_stream(self);
        futures_util::pin_mut!(stream);
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            bytes.extend_from_slice(chunk?.as_ref());
        }
        String::from_utf8(bytes).map_err(|error| {
            Failure::invalid_response("http", "Provider response is not UTF-8.").caused_by(&error)
        })
    }
    async fn bounded_json<T: DeserializeOwned + Send>(self) -> Result<T, Failure> {
        let text = self.bounded_text().await?;
        serde_json::from_str(&text).map_err(|error| {
            Failure::invalid_response("http", "Provider response is not the expected JSON.")
                .with_native(
                    battersea_model::Reporter::Provider,
                    serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text.clone())),
                )
                .caused_by(&error)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn chunk_boundaries_cannot_bypass_the_predecode_limit_and_failure_is_terminal() {
        for chunks in [vec![vec![0; 4]], vec![vec![0; 2], vec![0; 2]]] {
            let source =
                futures_util::stream::iter(chunks.into_iter().map(Ok::<_, std::io::Error>));
            let result = limit_stream(source, 3).collect::<Vec<_>>().await;
            assert!(result.last().unwrap().is_err());
            assert!(
                result
                    .iter()
                    .filter_map(|r| r.as_ref().ok())
                    .map(Vec::len)
                    .sum::<usize>()
                    <= 3
            );
        }
        let source = futures_util::stream::iter([Ok::<_, std::io::Error>(vec![0; 3])]);
        assert!(limit_stream(source, 3).collect::<Vec<_>>().await[0].is_ok());
    }
    /// contracts/failures.md, Cause: a body failure beneath an event stream is the cause, whole.
    #[test]
    fn an_event_stream_failure_keeps_the_body_failure_beneath_it() {
        let body = Failure::transport("http", "Provider response stream failed.")
            .caused_by_text("connection reset");
        let failure = event_stream_failure(
            Failure::transport("fixture", "Provider event stream failed."),
            eventsource_stream::EventStreamError::Transport(body.clone()),
        );
        assert_eq!(failure.cause, Some(battersea_model::Cause::Failure(body)));
    }
    /// contracts/failures.md, Cause: a body read failure keeps the text of its sources in order.
    #[tokio::test]
    async fn a_failed_body_read_carries_the_error_and_its_sources() {
        #[derive(Debug)]
        struct Decode(std::io::Error);
        impl std::fmt::Display for Decode {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("error decoding response body")
            }
        }
        impl std::error::Error for Decode {
            fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
                Some(&self.0)
            }
        }
        let source = futures_util::stream::iter([Err::<Vec<u8>, _>(Decode(
            std::io::Error::other("connection reset"),
        ))]);
        let failure = limit_stream(source, 3)
            .collect::<Vec<_>>()
            .await
            .remove(0)
            .err()
            .unwrap();
        assert_eq!(
            failure.cause,
            Some(battersea_model::Cause::Foreign(
                battersea_model::Detail::Present(vec![
                    "error decoding response body".into(),
                    "connection reset".into()
                ])
            ))
        );
    }
}
