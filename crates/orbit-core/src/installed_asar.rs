//! Read only authenticated, bounded JavaScript members from an installed ASAR.
//! The header digest comes from the signed Windows executable resource.
use crate::error::{Result, fail};
use std::io::{Read, Seek, SeekFrom};

pub(crate) fn claude_features(mut file: impl Read + Seek, header_sha256: &str) -> Result<String> {
    let bad = || fail("claude_configuration_target_unverified");
    let mut prefix = [0u8; 8];
    file.read_exact(&mut prefix).map_err(|_| bad())?;
    let header_bytes = u32::from_le_bytes(prefix[4..].try_into().unwrap()) as usize;
    if prefix[..4] != 4u32.to_le_bytes() || !(8..=16 * 1024 * 1024).contains(&header_bytes) {
        return Err(bad());
    }
    let mut header = vec![0; header_bytes];
    file.read_exact(&mut header).map_err(|_| bad())?;
    let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    let json = header
        .get(8..8usize.checked_add(len).ok_or_else(bad)?)
        .ok_or_else(bad)?;
    if crate::identity::sha256(json) != header_sha256 {
        return Err(bad());
    }
    let header: serde_json::Value = serde_json::from_slice(json).map_err(|_| bad())?;
    let entries = header
        .pointer("/files/.vite/files/build/files")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(bad)?;
    let mut observed = Vec::new();
    let mut count = 0;
    for (name, entry) in entries {
        if !name.starts_with("index") || !name.ends_with(".js") {
            continue;
        }
        let size = entry["size"].as_u64().ok_or_else(bad)?;
        let offset = entry["offset"]
            .as_str()
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(bad)?;
        count += 1;
        if count > 1024
            || size > 20 * 1024 * 1024
            || observed.len() as u64 + size > 32 * 1024 * 1024
            || entry["unpacked"] == true
            || entry["integrity"]["algorithm"] != "SHA256"
        {
            return Err(bad());
        }
        let expected = entry["integrity"]["hash"].as_str().ok_or_else(bad)?;
        let start = 8u64
            .checked_add(header_bytes as u64)
            .and_then(|v| v.checked_add(offset))
            .ok_or_else(bad)?;
        file.seek(SeekFrom::Start(start)).map_err(|_| bad())?;
        let mut bytes = vec![0; size as usize];
        file.read_exact(&mut bytes).map_err(|_| bad())?;
        if crate::identity::sha256(&bytes) != expected {
            return Err(bad());
        }
        observed.extend_from_slice(&bytes);
    }
    crate::client_compatibility::observe(
        std::io::Cursor::new(observed),
        crate::client_compatibility::CLAUDE_FEATURES,
        32 * 1024 * 1024,
    )
    .map_err(|_| bad())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authenticated_header_and_member_are_both_required() {
        let source = crate::client_compatibility::CLAUDE_FEATURES
            .join(" ")
            .into_bytes();
        let json = serde_json::to_vec(&serde_json::json!({"files":{".vite":{"files":{"build":{"files":{"index.js":{"size":source.len(),"offset":"0","integrity":{"algorithm":"SHA256","hash":crate::identity::sha256(&source)}}}}}}}})).unwrap();
        let mut bytes = Vec::new();
        bytes.extend(4u32.to_le_bytes());
        bytes.extend(((json.len() + 8) as u32).to_le_bytes());
        bytes.extend(((json.len() + 4) as u32).to_le_bytes());
        bytes.extend((json.len() as u32).to_le_bytes());
        bytes.extend(&json);
        bytes.extend(&source);
        let hash = crate::identity::sha256(&json);
        assert!(claude_features(std::io::Cursor::new(&bytes), &hash).is_ok());
        assert!(claude_features(std::io::Cursor::new(&bytes), &"0".repeat(64)).is_err());
        *bytes.last_mut().unwrap() ^= 1;
        assert!(claude_features(std::io::Cursor::new(&bytes), &hash).is_err());
    }
}
