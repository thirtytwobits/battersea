//! Copyright (c) Scott A Dixon
//! Battersea's flow, run, approval and MCP-connection operations, contributed to Clapham under
//! the `battersea` namespace over the host capabilities the application supplies. Every
//! operation runs under the caller's verified OAuth grant and needs the running engine.
pub mod codes;
mod host;
mod operations;
pub use host::*;
pub use operations::*;
