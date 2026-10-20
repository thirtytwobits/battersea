//! Copyright (c) Scott A Dixon
//! Streamable HTTP with bounded response bodies and disabled POST recovery.
use crate::{connection::State, wire, McpConfig, McpConnection, McpError};
use futures_util::{stream::BoxStream, StreamExt};
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
    timeout: Duration,
    state: Arc<State>,
}
type HttpError = StreamableHttpError<McpError>;
fn error(e: McpError) -> HttpError {
    StreamableHttpError::Client(e)
}
fn media_type(response: &reqwest::Response, expected: &str) -> bool {
    response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case(expected))
}
impl BoundedHttp {
    fn fail(&self, fault: McpError) -> HttpError {
        self.state.fail(fault.clone());
        error(fault)
    }
    fn new(config: &McpConfig, state: Arc<State>) -> Result<Self, McpError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(config.operation_timeout)
            .build()
            .map_err(|_| McpError::Configuration)?;
        Ok(Self {
            client,
            limit: config.limits.message_bytes,
            timeout: config.operation_timeout,
            state,
        })
    }
    fn request(
        &self,
        method: reqwest::Method,
        uri: &str,
        session: Option<&str>,
        auth: Option<String>,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> reqwest::RequestBuilder {
        let persistent = method == reqwest::Method::GET;
        let mut request = self
            .client
            .request(method, uri)
            .header("accept", "application/json, text/event-stream");
        if !persistent {
            request = request.timeout(self.timeout);
        }
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
        let state = self.state.clone();
        SseStream::from_bytes_stream(bytes)
            .map(move |event| {
                let mut event = event?;
                // Preserve raw catalogue records through the SDK's untagged result parser.
                if let Some(data) = &mut event.data {
                    if let Ok(mut value) = serde_json::from_str::<serde_json::Value>(data) {
                        if value.get("method").is_some() {
                            if let Err(error) = state.charge_incoming_bytes(data.len()) {
                                state.fail(error.clone());
                                return Err(SseError::Body(Box::new(error)));
                            }
                        }
                        if value.get("result").and_then(|r| r.get("tools")).is_some() {
                            value["result"] =
                                serde_json::json!({"batterseaCatalogue": value["result"].take()});
                            *data = value.to_string();
                        }
                    }
                }
                Ok(event)
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
            .map_err(|_| self.fail(McpError::Protocol))?;
        if media_type(&response, "text/event-stream") {
            return Ok(StreamableHttpPostResponse::Sse(
                self.stream(response),
                session,
            ));
        }
        if !media_type(&response, "application/json") {
            return Err(self.fail(McpError::Protocol));
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
        let message = wire::decode(&bytes).map_err(|fault| self.fail(fault))?;
        if let Err(fault) = self.state.observe_bytes(&message, bytes.len()) {
            self.state.fail(fault.clone());
            return Err(error(fault));
        }
        Ok(StreamableHttpPostResponse::Json(message, session))
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
        let response = tokio::time::timeout(self.timeout, request.send())
            .await
            .map_err(|_| error(McpError::Timeout))?
            .map_err(|_| error(McpError::Transport))?;
        if response.status().as_u16() == 405 {
            return Err(StreamableHttpError::ServerDoesNotSupportSse);
        }
        if !response.status().is_success() {
            return Err(error(McpError::Transport));
        }
        if !media_type(&response, "text/event-stream") {
            return Err(self.fail(McpError::Protocol));
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
        let state = State::new();
        let backend = BoundedHttp::new(&config, state.clone())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        response::{IntoResponse, Response},
        routing::get,
        Router,
    };

    async fn persistent_sse() -> Response {
        let events = futures_util::stream::unfold((), |()| async {
            tokio::time::sleep(Duration::from_millis(25)).await;
            Some((
                Ok::<_, std::convert::Infallible>(
                    axum::response::sse::Event::default()
                        .data(r#"{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}"#),
                ),
                (),
            ))
        });
        let mut response = axum::response::Sse::new(events).into_response();
        response.headers_mut().insert(
            "content-type",
            "Text/Event-Stream ; charset=utf-8".parse().unwrap(),
        );
        response
    }

    async fn slow_headers() -> Response {
        tokio::time::sleep(Duration::from_secs(5)).await;
        persistent_sse().await
    }

    async fn slow_body() -> Response {
        let chunks = futures_util::stream::once(async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok::<_, std::convert::Infallible>("{}")
        });
        let mut response = axum::body::Body::from_stream(chunks).into_response();
        response
            .headers_mut()
            .insert("content-type", "application/json".parse().unwrap());
        response
    }

    async fn padded_notification(
        axum::extract::Path(format): axum::extract::Path<String>,
    ) -> Response {
        let payload = format!(
            r#"{{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}}{}"#,
            " ".repeat(512)
        );
        if format == "sse" {
            (
                [("content-type", "text/event-stream")],
                format!("data: {payload}\n\n"),
            )
                .into_response()
        } else {
            ([("content-type", "application/json")], payload).into_response()
        }
    }

    async fn server() -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new()
            .route("/events", get(persistent_sse))
            .route(
                "/notification/{format}",
                axum::routing::post(padded_notification),
            )
            .route("/slow-headers", get(slow_headers))
            .route("/slow-body", axum::routing::post(slow_body));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async {
            axum::serve(listener, app).await.unwrap();
        });
        (endpoint, task)
    }

    fn backend() -> BoundedHttp {
        let mut config = McpConfig::new("remote", "scope");
        config.operation_timeout = Duration::from_millis(200);
        BoundedHttp::new(&config, State::new()).unwrap()
    }

    #[tokio::test]
    async fn persistent_sse_survives_operation_deadlines() {
        let (endpoint, task) = server().await;
        let backend = backend();
        let mut stream = backend
            .get_stream(
                format!("{endpoint}/events").into(),
                None,
                None,
                None,
                HashMap::new(),
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(600)).await;
        let event = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(event
            .data
            .unwrap()
            .contains("notifications/tools/list_changed"));
        assert_eq!(backend.state.check(), Ok(()));
        task.abort();
    }

    #[tokio::test]
    async fn padded_http_notifications_charge_received_payload_bytes() {
        let (endpoint, task) = server().await;
        for format in ["json", "sse"] {
            let backend = backend();
            backend
                .state
                .charge_incoming_bytes(64 * 1024 * 1024 - 128)
                .unwrap();
            let message =
                serde_json::from_value(serde_json::json!({"jsonrpc":"2.0","id":1,"method":"ping"}))
                    .unwrap();
            let response = backend
                .post_message(
                    format!("{endpoint}/notification/{format}").into(),
                    message,
                    None,
                    None,
                    HashMap::new(),
                )
                .await;
            if format == "sse" {
                let StreamableHttpPostResponse::Sse(mut stream, _) = response.unwrap() else {
                    panic!("expected SSE response");
                };
                assert!(stream.next().await.unwrap().is_err());
            } else {
                assert!(matches!(
                    response,
                    Err(StreamableHttpError::Client(McpError::Limit))
                ));
            }
            assert_eq!(backend.state.check(), Err(McpError::Limit));
        }
        task.abort();
    }

    #[tokio::test]
    async fn finite_requests_and_get_headers_keep_deadlines() {
        let (endpoint, task) = server().await;
        let backend = backend();
        let get = tokio::time::timeout(
            Duration::from_secs(1),
            backend.get_stream(
                format!("{endpoint}/slow-headers").into(),
                None,
                None,
                None,
                HashMap::new(),
            ),
        )
        .await
        .unwrap();
        assert!(matches!(
            get,
            Err(StreamableHttpError::Client(McpError::Timeout))
        ));
        let message =
            serde_json::from_value(serde_json::json!({"jsonrpc":"2.0","id":1,"method":"ping"}))
                .unwrap();
        let post = tokio::time::timeout(
            Duration::from_secs(1),
            backend.post_message(
                format!("{endpoint}/slow-body").into(),
                message,
                None,
                None,
                HashMap::new(),
            ),
        )
        .await
        .unwrap();
        assert!(post.is_err());
        task.abort();
    }
}
