use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub type ContentMask<'a> = dyn Fn(&[u8]) -> String + 'a;

/// Metadata describes the original bytes; captured text has passed the host mask.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Content {
    pub sha256: String,
    pub byte_length: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "String")]
    pub masked: Option<String>,
}
impl Content {
    pub fn observe(bytes: &[u8], mask: Option<&ContentMask<'_>>) -> Self {
        Self {
            sha256: format!("{:x}", Sha256::digest(bytes)),
            byte_length: bytes.len() as u64,
            masked: mask.map(|mask| mask(bytes)),
        }
    }
}
