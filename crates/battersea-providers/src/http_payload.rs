//! Bound raw HTTP bytes before JSON or SSE decoding retains them.
use crate::EngineAdapterRequestError;
use battersea_model::adapter::payload::PROVIDER_PAYLOAD_BYTES;
use futures_util::{Stream, StreamExt};
use serde::de::DeserializeOwned;

pub(crate) fn bounded_stream(
    response: reqwest::Response,
) -> impl Stream<Item = Result<impl AsRef<[u8]>, EngineAdapterRequestError>> + Send {
    limit_stream(response.bytes_stream(), PROVIDER_PAYLOAD_BYTES)
}

fn limit_stream<T: AsRef<[u8]>, E: std::fmt::Display>(
    stream: impl Stream<Item = Result<T, E>>,
    limit: usize,
) -> impl Stream<Item = Result<T, EngineAdapterRequestError>> {
    stream.scan(Some(limit), |remaining, chunk| {
        let value = match (remaining.as_mut(), chunk) {
            (None, _) => None,
            (Some(left), Ok(chunk)) if chunk.as_ref().len() <= *left => {
                *left -= chunk.as_ref().len();
                Some(Ok(chunk))
            }
            (Some(_), Ok(_)) => {
                *remaining = None;
                Some(Err(EngineAdapterRequestError::invalid_response(
                    "http",
                    "Provider response exceeds its encoded-byte limit.",
                )))
            }
            (Some(_), Err(error)) => {
                *remaining = None;
                Some(Err(EngineAdapterRequestError::transport(
                    "http",
                    error.to_string(),
                )))
            }
        };
        std::future::ready(value)
    })
}

#[allow(clippy::double_must_use)] // async_trait marks its boxed Future must_use.
#[async_trait::async_trait]
pub(crate) trait BoundedResponse {
    async fn bounded_text(self) -> Result<String, EngineAdapterRequestError>;
    async fn bounded_json<T: DeserializeOwned + Send>(self)
        -> Result<T, EngineAdapterRequestError>;
}
#[async_trait::async_trait]
impl BoundedResponse for reqwest::Response {
    async fn bounded_text(self) -> Result<String, EngineAdapterRequestError> {
        let stream = bounded_stream(self);
        futures_util::pin_mut!(stream);
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            bytes.extend_from_slice(chunk?.as_ref());
        }
        String::from_utf8(bytes)
            .map_err(|error| EngineAdapterRequestError::invalid_response("http", error.to_string()))
    }
    async fn bounded_json<T: DeserializeOwned + Send>(
        self,
    ) -> Result<T, EngineAdapterRequestError> {
        serde_json::from_str(&self.bounded_text().await?)
            .map_err(|error| EngineAdapterRequestError::invalid_response("http", error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn chunk_boundaries_cannot_bypass_the_predecode_limit_and_failure_is_terminal() {
        for chunks in [vec![vec![0; 4]], vec![vec![0; 2], vec![0; 2]]] {
            let source = futures_util::stream::iter(chunks.into_iter().map(Ok::<_, String>));
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
        let source = futures_util::stream::iter([Ok::<_, String>(vec![0; 3])]);
        assert!(limit_stream(source, 3).collect::<Vec<_>>().await[0].is_ok());
    }
}
