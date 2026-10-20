//! Copyright (c) Scott A Dixon
//! Streamable HTTP with bounded response bodies and disabled POST recovery.
use crate::{connection::State, wire, McpConfig, McpConnection, McpError};
use futures_util::{stream::BoxStream, StreamExt, TryStreamExt};
use http::{HeaderName, HeaderValue};
use rmcp::{
    model::{ClientJsonRpcMessage, ServerJsonRpcMessage},
    transport::{common::client_side_sse::SseRetryPolicy, streamable_http_client::*, Transport},
    RoleClient,
};
use sse_stream::{Error as SseError, Sse, SseStream};
use std::{collections::HashMap, sync::Arc, time::Duration};

/// Endpoint and host-resolved bearer credential. Debug formatting intentionally omits both.
pub struct McpHttpOptions {
    pub endpoint: String,
    pub bearer_token: Option<String>,
}
impl McpHttpOptions {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            bearer_token: None,
        }
    }
}
impl std::fmt::Debug for McpHttpOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpHttpOptions").finish_non_exhaustive()
    }
}
#[derive(Clone)]
struct BoundedHttp {
    client: reqwest::Client,
    limit: usize,
    state: Arc<State>,
}
type HttpError = StreamableHttpError<McpError>;
fn error(e: McpError) -> HttpError {
    StreamableHttpError::Client(e)
}
impl BoundedHttp {
    fn request(
        &self,
        method: reqwest::Method,
        uri: &str,
        session: Option<&str>,
        auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> reqwest::RequestBuilder {
        let mut request = self
            .client
            .request(method, uri)
            .header("accept", "application/json, text/event-stream");
        if let Some(session) = session {
            request = request.header("mcp-session-id", session);
        }
        if let Some(auth) = auth {
            request = request.bearer_auth(auth);
        }
        for (name, value) in headers {
            request = request.header(name, value);
        }
        request
    }
    async fn send(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response, HttpError> {
        let response = request
            .send()
            .await
            .map_err(|_| error(McpError::Transport))?;
        if !response.status().is_success() {
            return Err(error(McpError::Transport));
        }
        Ok(response)
    }
    fn stream(&self, response: reqwest::Response) -> BoxStream<'static, Result<Sse, SseError>> {
        let limit = self.limit;
        let state = self.state.clone();
        // The entire response is bounded, which also bounds each event before SSE parsing.
        let bytes = response.bytes_stream().scan(0usize, move |count, chunk| {
            let result = chunk.map_err(|_| McpError::Transport).and_then(|chunk| {
                *count = count.saturating_add(chunk.len());
                if *count > limit {
                    Err(McpError::Limit)
                } else {
                    Ok(chunk)
                }
            });
            if let Err(error) = &result {
                state.fail(error.clone());
            }
            std::future::ready(Some(result))
        });
        SseStream::from_bytes_stream(bytes)
            .map_ok(|mut event| {
                // Preserve raw catalogue records through the SDK's untagged result parser.
                if let Some(data) = &mut event.data {
                    if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(data) {
                        if value.get("result").and_then(|r| r.get("tools")).is_some() {
                            value["result"] =
                                serde_json::json!({"batterseaCatalogue": value["result"].take()});
                            *data = value.to_string();
                        }
                    }
                }
                event
            })
            .boxed()
    }
}
impl StreamableHttpClient for BoundedHttp {
    type Error = McpError;
    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, HttpError> {
        let body = wire::encode(&message, self.limit).map_err(error)?;
        let response = self
            .send(
                self.request(
                    reqwest::Method::POST,
                    &uri,
                    session_id.as_deref(),
                    auth,
                    headers,
                )
                .header("content-type", "application/json")
                .body(body),
            )
            .await?;
        if matches!(response.status().as_u16(), 202 | 204) {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        let session = response
            .headers()
            .get("mcp-session-id")
            .map(|h| h.to_str().map(str::to_owned))
            .transpose()
            .map_err(|_| error(McpError::Protocol))?;
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("");
        if content_type.split(';').next() == Some("text/event-stream") {
            return Ok(StreamableHttpPostResponse::Sse(
                self.stream(response),
                session,
            ));
        }
        if content_type.split(';').next() != Some("application/json") {
            return Err(error(McpError::Protocol));
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| error(McpError::Transport))?;
            if chunk.len() > self.limit.saturating_sub(bytes.len()) {
                self.state.fail(McpError::Limit);
                return Err(error(McpError::Limit));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(StreamableHttpPostResponse::Json(
            wire::decode(&bytes).map_err(error)?,
            session,
        ))
    }
    async fn delete_session(
        &self,
        uri: Arc<str>,
        session: Arc<str>,
        auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), HttpError> {
        self.send(self.request(reqwest::Method::DELETE, &uri, Some(&session), auth, headers))
            .await?;
        Ok(())
    }
    async fn get_stream(
        &self,
        uri: Arc<str>,
        session: Option<Arc<str>>,
        last: Option<String>,
        auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, HttpError> {
        let mut request = self.request(
            reqwest::Method::GET,
            &uri,
            session.as_deref(),
            auth,
            headers,
        );
        if let Some(last) = last {
            request = request.header("last-event-id", last);
        }
        let response = request
            .send()
            .await
            .map_err(|_| error(McpError::Transport))?;
        if response.status().as_u16() == 405 {
            return Err(StreamableHttpError::ServerDoesNotSupportSse);
        }
        if !response.status().is_success() {
            return Err(error(McpError::Transport));
        }
        if response
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.split(';').next())
            != Some("text/event-stream")
        {
            return Err(error(McpError::Protocol));
        }
        Ok(self.stream(response))
    }
}
#[derive(Debug)]
struct NoRetry;
impl SseRetryPolicy for NoRetry {
    fn retry(&self, _: usize) -> Option<Duration> {
        None
    }
}
struct Guard<T> {
    inner: T,
    state: Arc<State>,
    notifications: usize,
}
impl<T: Transport<RoleClient> + 'static> Transport<RoleClient> for Guard<T> {
    type Error = T::Error;
    fn send(
        &mut self,
        message: ClientJsonRpcMessage,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner.send(message)
    }
    async fn receive(&mut self) -> Option<ServerJsonRpcMessage> {
        let message = self.inner.receive().await?;
        if let Err(error) = self.state.observe(&message, self.notifications) {
            self.state.fail(error);
            return None;
        }
        Some(message)
    }
    async fn close(&mut self) -> Result<(), Self::Error> {
        self.inner.close().await
    }
}
impl McpConnection {
    /// Connect a host-approved endpoint. Redirects, POST retries and session reinitialisation are disabled.
    pub async fn http(options: McpHttpOptions, config: McpConfig) -> Result<Self, McpError> {
        config.validate()?;
        let url = reqwest::Url::parse(&options.endpoint).map_err(|_| McpError::Configuration)?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(McpError::Configuration);
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(config.operation_timeout)
            .build()
            .map_err(|_| McpError::Configuration)?;
        let state = State::new();
        let backend = BoundedHttp {
            client,
            limit: config.limits.message_bytes,
            state: state.clone(),
        };
        let mut transport_config = StreamableHttpClientTransportConfig::with_uri(options.endpoint);
        transport_config.auth_header = options.bearer_token;
        transport_config.reinit_on_expired_session = false;
        transport_config.retry_config = Arc::new(NoRetry);
        transport_config.max_concurrent_requests = config.limits.in_flight;
        transport_config.channel_buffer_capacity = config.limits.in_flight;
        transport_config.max_sse_event_size = config.limits.message_bytes;
        transport_config.control_request_timeout = config.operation_timeout;
        let transport = Guard {
            inner: StreamableHttpClientTransport::with_client(backend, transport_config),
            state: state.clone(),
            notifications: config.limits.notifications,
        };
        Self::start(transport, config, state).await
    }
}
