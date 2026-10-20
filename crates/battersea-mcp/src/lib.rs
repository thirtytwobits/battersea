//! Copyright (c) Scott A Dixon
//! Host-owned MCP connections and pinned tool catalogues for Battersea.
mod binding;
mod connection;
#[cfg(feature = "http")]
mod http;
#[cfg(feature = "stdio")]
mod stdio;
mod wire;

pub use binding::{McpCatalogue, McpDiagnostic, McpSelection, McpToolBinding, McpToolExecutor};
pub use connection::{McpConfig, McpConnection, McpError, McpLimits};
#[cfg(feature = "http")]
pub use http::McpHttpOptions;
