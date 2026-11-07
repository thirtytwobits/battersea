//! Copyright (c) Scott A Dixon
//!
//! What every kit needs: the [`Kit`] descriptor with its name checks, the in-memory
//! [`ReferenceHost`] on a manual clock, and, behind the `conformance` feature, the
//! differential harness an application runs against its own host. `contracts/kits.md`
//! defines the parts of a kit and the rules this crate enforces.
pub use battersea_model::clock;
pub use battersea_model::{Clock, ManualClock};
pub mod error;
pub use error::{NodeError, NodeResult};
pub mod kit;
pub use kit::{pascal_case, Kit, KitError, GENERIC_KIT, SHARED_TOKEN_TYPES};
pub mod reference;
pub use reference::{ReferenceError, ReferenceHost, ReferenceState};
#[cfg(feature = "conformance")]
pub mod conformance;
pub mod text;
