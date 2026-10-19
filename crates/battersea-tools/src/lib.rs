//! Copyright (c) Scott A Dixon
//! Application tools as Model Context Protocol tool records, the agent profiles permitted to
//! call them, and the registry that offers them to providers and runs their calls.
mod record;
mod registry;
pub mod schema;
pub use record::*;
pub use registry::*;
