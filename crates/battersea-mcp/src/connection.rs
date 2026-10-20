//! Copyright (c) Scott A Dixon
//! Connection lifecycle, admission limits and dispatch certainty.
use crate::{binding::Published, McpCatalogue};
use rmcp::{
    model::*,
    service::{NotificationContext, RequestContext, RunningService, ServiceError},
    ClientHandler, RoleClient,
};
#[cfg(any(feature = "stdio", feature = "http"))]
use rmcp::{service::ClientLifecycleMode, transport::Transport};
#[cfg(any(feature = "stdio", feature = "http"))]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{
    sync::{atomic::AtomicU64, Arc, Mutex},
    time::Duration,
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

/// Resource bounds applied before transport buffering and catalogue retention.
#[derive(Clone, Debug)]
pub struct McpLimits {
    pub message_bytes: usize,
    pub catalogue_bytes: usize,
    pub schema_bytes: usize,
    pub tools: usize,
    pub pages: usize,
    pub in_flight: usize,
    pub waiting: usize,
    /// Maximum incoming requests and notifications over a connection's lifetime.
    pub notifications: usize,
}
impl Default for McpLimits {
    fn default() -> Self {
        Self {
            message_bytes: 1024 * 1024,
            catalogue_bytes: 8 * 1024 * 1024,
            schema_bytes: 128 * 1024,
            tools: 512,
            pages: 32,
            in_flight: 4,
            waiting: 16,
            notifications: 4096,
        }
    }
}

/// Host authority and operation deadline. Reconnect with a new configuration after credentials change.
#[derive(Clone, Debug)]
pub struct McpConfig {
    pub server_id: String,
    pub credential_scope_revision: String,
    pub operation_timeout: Duration,
    pub limits: McpLimits,
}
impl McpConfig {
    pub fn new(server_id: impl Into<String>, credential_scope_revision: impl Into<String>) -> Self {
        Self {
            server_id: server_id.into(),
            credential_scope_revision: credential_scope_revision.into(),
            operation_timeout: Duration::from_secs(30),
            limits: McpLimits::default(),
        }
    }
    #[cfg(any(feature = "stdio", feature = "http"))]
    pub(crate) fn validate(&self) -> Result<(), McpError> {
        let l = &self.limits;
        if self.server_id.is_empty()
            || self.server_id.len() > 128
            || !self
                .server_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || self.credential_scope_revision.trim().is_empty()
            || self.credential_scope_revision.len() > 256
            || self.operation_timeout.is_zero()
            || self.operation_timeout > Duration::from_secs(3600)
            || l.message_bytes == 0
            || l.catalogue_bytes == 0
            || l.schema_bytes == 0
            || l.message_bytes > 8 * 1024 * 1024
            || l.catalogue_bytes > 64 * 1024 * 1024
            || l.schema_bytes > l.message_bytes
            || l.tools == 0
            || l.tools > 4096
            || l.pages == 0
            || l.pages > 1024
            || l.in_flight == 0
            || l.in_flight > 32
            || l.waiting > 1024
            || l.notifications == 0
            || l.notifications > 16384
            || l.message_bytes.saturating_mul(l.in_flight + l.waiting + 2) > 64 * 1024 * 1024
        {
            return Err(McpError::Configuration);
        }
        Ok(())
    }
}

/// Bounded diagnostics. Remote messages, credentials and response data are excluded.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum McpError {
    #[error("Invalid MCP configuration")]
    Configuration,
    #[error("MCP resource limit exceeded")]
    Limit,
    #[error("MCP operation deadline exceeded")]
    Timeout,
    #[error("MCP transport closed or failed")]
    Transport,
    #[error("Invalid or unsupported MCP response")]
    Protocol,
    #[error("MCP protocol invocation error ({0})")]
    Invocation(i32),
    #[error("MCP catalogue or selected definition changed")]
    Stale,
    #[error("MCP tool selection is invalid or collides with a registered tool")]
    Selection,
    #[error("MCP operation queue is full")]
    Busy,
    #[error("MCP interaction requires an unsupported host capability")]
    UnsupportedInteraction,
}
impl McpError {
    pub(crate) fn from_service(error: ServiceError) -> Self {
        match error {
            ServiceError::McpError(e) => Self::Invocation(e.code.0),
            ServiceError::UnexpectedResponse => Self::Protocol,
            ServiceError::Timeout { .. } => Self::Timeout,
            _ => Self::Transport,
        }
    }
}

pub(crate) struct State {
    pub epoch: AtomicU64,
    pub fault: Mutex<Option<McpError>>,
    #[cfg(any(feature = "stdio", feature = "http"))]
    incoming: AtomicUsize,
    #[cfg(any(feature = "stdio", feature = "http"))]
    incoming_bytes: AtomicUsize,
    pub stop: CancellationToken,
}
impl State {
    #[cfg(any(feature = "stdio", feature = "http"))]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            epoch: AtomicU64::new(0),
            fault: Mutex::new(None),
            incoming: AtomicUsize::new(0),
            incoming_bytes: AtomicUsize::new(0),
            stop: CancellationToken::new(),
        })
    }
    pub fn fail(&self, error: McpError) {
        *self.fault.lock().unwrap() = Some(error);
        self.stop.cancel();
    }
    pub fn check(&self) -> Result<(), McpError> {
        if let Some(error) = self.fault.lock().unwrap().clone() {
            return Err(error);
        }
        if self.stop.is_cancelled() {
            return Err(McpError::Transport);
        }
        Ok(())
    }
    #[cfg(any(feature = "stdio", feature = "http"))]
    pub fn charge_incoming_bytes(&self, bytes: usize) -> Result<(), McpError> {
        const BUDGET: usize = 64 * 1024 * 1024;
        if bytes > BUDGET || self.incoming_bytes.fetch_add(bytes, Ordering::SeqCst) > BUDGET - bytes
        {
            return Err(McpError::Limit);
        }
        Ok(())
    }
    #[cfg(any(feature = "stdio", feature = "http"))]
    pub fn observe_bytes(
        &self,
        message: &ServerJsonRpcMessage,
        bytes: usize,
    ) -> Result<(), McpError> {
        if matches!(
            message,
            ServerJsonRpcMessage::Request(_) | ServerJsonRpcMessage::Notification(_)
        ) {
            self.charge_incoming_bytes(bytes)?;
        }
        Ok(())
    }
    #[cfg(any(feature = "stdio", feature = "http"))]
    pub fn observe(&self, message: &ServerJsonRpcMessage, limit: usize) -> Result<(), McpError> {
        if matches!(
            message,
            ServerJsonRpcMessage::Request(_) | ServerJsonRpcMessage::Notification(_)
        ) && self.incoming.fetch_add(1, Ordering::SeqCst) >= limit
        {
            return Err(McpError::Limit);
        }
        // Invalidate before the service schedules its notification handler.
        if matches!(message, ServerJsonRpcMessage::Notification(n) if matches!(n.notification, ServerNotification::ToolListChangedNotification(_)))
        {
            self.epoch.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

pub(crate) struct Handler;
#[allow(deprecated)]
impl ClientHandler for Handler {
    async fn list_roots(
        &self,
        _: RequestContext<RoleClient>,
    ) -> Result<ListRootsResult, ErrorData> {
        Err(ErrorData::method_not_found::<ListRootsRequestMethod>())
    }
    async fn create_elicitation(
        &self,
        _: ElicitRequestParams,
        _: RequestContext<RoleClient>,
    ) -> Result<ElicitResult, ErrorData> {
        Err(ErrorData::new(
            ErrorCode::METHOD_NOT_FOUND,
            "Unsupported elicitation",
            None,
        ))
    }
    async fn on_tool_list_changed(&self, _: NotificationContext<RoleClient>) {}
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("battersea-mcp", env!("CARGO_PKG_VERSION")),
        )
    }
}

pub(crate) struct Inner {
    pub service: RunningService<RoleClient, Handler>,
    pub config: McpConfig,
    pub state: Arc<State>,
    pub active: Semaphore,
    pub admitted: Semaphore,
    pub discovery: tokio::sync::Mutex<()>,
    pub published: Mutex<Option<Published>>,
    pub revision: AtomicU64,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.state.stop.cancel();
    }
}

/// A connection shared by pinned bindings. The host owns endpoint and process authority.
#[derive(Clone)]
pub struct McpConnection(pub(crate) Arc<Inner>);
impl McpConnection {
    #[cfg(any(feature = "stdio", feature = "http"))]
    pub(crate) async fn start<T: Transport<RoleClient> + 'static>(
        transport: T,
        config: McpConfig,
        state: Arc<State>,
    ) -> Result<Self, McpError> {
        let service = tokio::time::timeout(
            config.operation_timeout,
            rmcp::service::serve_client_with_lifecycle_and_ct(
                Handler,
                transport,
                ClientLifecycleMode::Discover {
                    preferred_versions: vec![ProtocolVersion::V_2026_07_28],
                },
                state.stop.clone(),
            ),
        )
        .await;
        let service = match service {
            Ok(Ok(service)) => service,
            result => {
                let error = state.check().err().unwrap_or(if result.is_err() {
                    McpError::Timeout
                } else {
                    McpError::Protocol
                });
                state.fail(error.clone());
                return Err(error);
            }
        };
        Ok(Self(Arc::new(Inner {
            active: Semaphore::new(config.limits.in_flight),
            admitted: Semaphore::new(config.limits.in_flight + config.limits.waiting),
            config,
            service,
            state,
            discovery: tokio::sync::Mutex::new(()),
            published: Mutex::new(None),
            revision: AtomicU64::new(0),
        })))
    }
    /// Stop transport work. Process termination remains the host's responsibility.
    pub fn close(&self) {
        self.0.state.stop.cancel();
    }
    pub fn server_id(&self) -> &str {
        &self.0.config.server_id
    }
    pub fn credential_scope_revision(&self) -> &str {
        &self.0.config.credential_scope_revision
    }
    /// Discover an uncached, bounded catalogue. Failure invalidates prior bindings.
    pub async fn discover(&self) -> Result<McpCatalogue, McpError> {
        McpCatalogue::discover(self.clone()).await
    }
    pub(crate) fn check(&self) -> Result<(), McpError> {
        self.0.state.check()?;
        if self.0.service.is_closed() {
            return Err(McpError::Transport);
        }
        Ok(())
    }
}
