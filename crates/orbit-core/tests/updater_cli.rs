use base64::{Engine, engine::general_purpose::STANDARD};
use orbit_core::{images::bounded_read, updates};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SyntheticFixture {
    public_key: String,
    other_public_key: String,
    signature: String,
    unversioned_signature: String,
    version: String,
    artifact_base64: String,
}

/// Run only with Root's explicitly created public, synthetic CLI fixture.
/// This test reads no signing private key, vault, channel or network.
#[test]
#[ignore = "requires the project-local locked CLI synthetic signer probe"]
fn synthetic_cli_signed_artifact() {
    assert!(updates::compiled_trust().is_none());
    let project_tmp = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/tmp")
        .canonicalize()
        .unwrap();
    let path = PathBuf::from(std::env::var_os("ORBIT_SYNTHETIC_UPDATER_FIXTURE").unwrap())
        .canonicalize()
        .unwrap();
    assert!(path.starts_with(project_tmp));
    let fixture: SyntheticFixture =
        serde_json::from_slice(&bounded_read(&path, 1024 * 1024).unwrap()).unwrap();
    assert!(!fixture.version.is_empty() && fixture.version.len() <= 64);
    for value in [
        &fixture.public_key,
        &fixture.other_public_key,
        &fixture.signature,
        &fixture.unversioned_signature,
    ] {
        assert!(!value.is_empty() && value.len() <= 16 * 1024);
    }
    let artifact = STANDARD.decode(&fixture.artifact_base64).unwrap();
    assert!(!artifact.is_empty() && artifact.len() <= 512 * 1024);
    let verify = |key: &str, signature: &str, version: &str, bytes: &[u8]| {
        updates::verify_artifact(key, signature, version, bytes)
    };
    verify(
        &fixture.public_key,
        &fixture.signature,
        &fixture.version,
        &artifact,
    )
    .unwrap();
    let mut changed = artifact.clone();
    changed[0] ^= 1;
    assert!(
        verify(
            &fixture.public_key,
            &fixture.signature,
            &fixture.version,
            &changed
        )
        .is_err()
    );
    assert!(
        verify(
            &fixture.other_public_key,
            &fixture.signature,
            &fixture.version,
            &artifact
        )
        .is_err()
    );
    let other_version = if fixture.version == "9.9.9" {
        "9.9.8"
    } else {
        "9.9.9"
    };
    assert!(
        verify(
            &fixture.public_key,
            &fixture.signature,
            other_version,
            &artifact
        )
        .is_err()
    );
    assert!(
        verify(
            &fixture.public_key,
            &fixture.unversioned_signature,
            &fixture.version,
            &artifact
        )
        .is_err()
    );
    let signature = String::from_utf8(STANDARD.decode(&fixture.signature).unwrap()).unwrap();
    assert!(signature.contains("trusted comment: "));
    let tampered =
        STANDARD.encode(signature.replace("trusted comment: ", "trusted comment: modified "));
    assert!(verify(&fixture.public_key, &tampered, &fixture.version, &artifact).is_err());
    for malformed in ["", "%%%", "Zg", "Zm9v"] {
        assert!(verify(&fixture.public_key, malformed, &fixture.version, &artifact).is_err());
        assert!(verify(malformed, &fixture.signature, &fixture.version, &artifact).is_err());
    }
    assert!(
        verify(
            &fixture.public_key,
            &fixture.signature,
            &fixture.version,
            &[]
        )
        .is_err()
    );
}
