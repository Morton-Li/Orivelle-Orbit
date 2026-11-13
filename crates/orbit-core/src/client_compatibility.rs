//! Version-independent observations of the configuration features Orbit uses.
//! These markers describe a configuration dialect, not proof of client loading.
use crate::error::{Result, fail};
use sha2::{Digest, Sha256};
use std::io::Read;

pub(crate) const CODEX_FEATURES: &[&str] = &[
    "model_providers",
    "model_catalog_json",
    "wire_api",
    "responses",
    "base_url",
    "refresh_interval_ms",
    "timeout_ms",
];
pub(crate) const CLAUDE_FEATURES: &[&str] = &[
    "configLibrary",
    "_meta.json",
    "appliedId",
    "deploymentMode",
    "inferenceProvider",
    "inferenceGatewayBaseUrl",
    "inferenceCredentialHelper",
    "inferenceCredentialHelperArgs",
    "inferenceCredentialKind",
    "inferenceModels",
];

/// Bound memory and work, including markers split between read boundaries.
pub(crate) fn observe(mut reader: impl Read, features: &[&str], limit: u64) -> Result<String> {
    let mut found = vec![false; features.len()];
    let overlap = features.iter().map(|s| s.len()).max().unwrap_or(1) - 1;
    let mut tail = Vec::new();
    let mut chunk = [0u8; 128 * 1024];
    let mut total = 0u64;
    let mut digest = Sha256::new();
    loop {
        let read = reader
            .read(&mut chunk)
            .map_err(|_| fail("configuration_unreadable"))?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > limit {
            return Err(fail("configuration_unreadable"));
        }
        tail.extend_from_slice(&chunk[..read]);
        // Only ASCII feature names are inspected. Replacement of invalid UTF-8
        // cannot manufacture or remove one of these names in executable bytes.
        let text = String::from_utf8_lossy(&tail);
        let mut discovered = false;
        for (index, feature) in features.iter().enumerate() {
            if !found[index] {
                found[index] = text.contains(feature);
                discovered |= found[index];
            }
        }
        if discovered {
            digest.update(&tail);
        }
        if !found.contains(&false) {
            break;
        }
        let retained = tail.len().saturating_sub(overlap);
        tail.drain(..retained);
    }
    if total == 0 || found.contains(&false) {
        return Err(fail("schema_unverified"));
    }
    Ok(format!("{:x}", digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dialect_survives_unrelated_application_changes_but_requires_every_feature() {
        for features in [CODEX_FEATURES, CLAUDE_FEATURES] {
            let source = features.join("\0");
            assert!(observe(source.as_bytes(), features, 1024).is_ok());
            let upgraded = format!("major-version-999\0{source}\0new-feature");
            assert!(observe(upgraded.as_bytes(), features, 1024).is_ok());
            assert!(observe(features[1..].join("\0").as_bytes(), features, 1024).is_err());
            assert!(observe(source.as_bytes(), features, 1).is_err());
        }
    }
    #[test]
    fn handles_markers_crossing_chunks_and_invalid_binary_utf8() {
        let mut bytes = vec![0xff; 128 * 1024 - 3];
        bytes.extend_from_slice(b"model_catalog_json");
        assert!(observe(bytes.as_slice(), &["model_catalog_json"], 256 * 1024).is_ok());
    }
}
