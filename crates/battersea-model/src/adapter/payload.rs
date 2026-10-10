//! Finite encoded-payload admission for provider buffers and tool transcripts.
use super::EngineAdapterRequestError;
use serde::Serialize;
use std::io::{self, Write};

/// Maximum encoded body, transcript or tool event retained by a built-in provider.
pub const PROVIDER_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;

/// Count encoded bytes without allocating a second serialised copy.
pub fn check_payload<T: Serialize + ?Sized>(
    value: &T,
    provider: &str,
) -> Result<usize, EngineAdapterRequestError> {
    check_limit(value, PROVIDER_PAYLOAD_BYTES).map_err(|error| {
        EngineAdapterRequestError::invalid_response(
            provider,
            format!(
                "Provider payload exceeds its encoded-byte limit or cannot be encoded: {error}"
            ),
        )
    })
}

fn check_limit<T: Serialize + ?Sized>(value: &T, limit: usize) -> serde_json::Result<usize> {
    struct Counter(usize);
    impl Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 = self
                .0
                .checked_sub(bytes.len())
                .ok_or_else(|| io::Error::other("encoded-byte capacity exceeded"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(limit);
    serde_json::to_writer(&mut counter, value)?;
    Ok(limit - counter.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admission_counts_json_escaping_and_rejects_before_allocating_an_encoded_copy() {
        let value = "\n\\\"";
        let bytes = serde_json::to_vec(value).unwrap().len();
        assert!(check_limit(value, bytes).is_ok());
        assert!(check_limit(value, bytes - 1).is_err());
    }
}
