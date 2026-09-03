//! Release verification: minisign signature over `checksums.txt`, then the
//! SHA-256 of a downloaded asset against that file.
//!
//! Every function takes the public key as a parameter so tests can use the
//! fixture key in `cli/tests/fixtures/selfhost/`; production callers pass
//! [`super::keys::MINISIGN_PUBLIC_KEY`]. Errors name the check that failed
//! (`Signature check failed` / `Checksum check failed`) because installers
//! surface them verbatim and must never delete anything on failure.

use std::fmt;
use std::fs;
use std::io::Read;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use minisign_verify::{PublicKey, Signature};
use sha2::{Digest, Sha256};

/// Which verification step rejected the artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// The minisign signature does not validate for the checksums file.
    Signature(String),
    /// The asset's SHA-256 does not match (or is missing from) `checksums.txt`.
    Checksum(String),
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerifyError::Signature(detail) => write!(f, "Signature check failed: {detail}"),
            VerifyError::Checksum(detail) => write!(f, "Checksum check failed: {detail}"),
        }
    }
}

impl std::error::Error for VerifyError {}

/// Verify a minisign signature (`checksums.txt.minisig` contents) over
/// `checksums` with the base64 public key. Legacy (non-prehashed) signatures
/// are rejected; `minisign -S` has produced prehashed signatures since 0.7.
pub fn verify_signature(checksums: &[u8], signature: &str, public_key_b64: &str) -> Result<()> {
    let public_key = PublicKey::from_base64(public_key_b64)
        .map_err(|error| anyhow!("Embedded minisign public key is invalid: {error}"))?;
    let signature = Signature::decode(signature).map_err(|error| {
        VerifyError::Signature(format!("signature file is malformed ({error})"))
    })?;
    public_key
        .verify(checksums, &signature, false)
        .map_err(|error| {
            VerifyError::Signature(format!(
                "checksums.txt is not signed by the Arete release key ({error})"
            ))
        })?;
    Ok(())
}

/// Parse `checksums.txt` lines (`<sha256>  <asset>`; a leading `*` before the
/// name, as written by `sha256sum -b`, is tolerated) into `(sha256, name)`.
pub fn parse_checksums(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let sha = parts.next()?;
            let name = parts.next()?.trim_start_matches('*');
            if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) || name.is_empty() {
                return None;
            }
            Some((sha.to_ascii_lowercase(), name.to_string()))
        })
        .collect()
}

/// The SHA-256 recorded for `asset_name` in `checksums.txt`.
pub fn expected_sha256(checksums: &str, asset_name: &str) -> Result<String> {
    parse_checksums(checksums)
        .into_iter()
        .find(|(_, name)| name == asset_name)
        .map(|(sha, _)| sha)
        .ok_or_else(|| {
            VerifyError::Checksum(format!("checksums.txt has no entry for {asset_name}")).into()
        })
}

/// Lowercase hex SHA-256 of a file, streamed.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        fs::File::open(path).with_context(|| format!("Failed to open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Check that `asset` hashes to the entry for `asset_name` in `checksums`.
pub fn verify_sha256(checksums: &str, asset: &Path, asset_name: &str) -> Result<()> {
    let expected = expected_sha256(checksums, asset_name)?;
    let actual = sha256_file(asset)?;
    if actual != expected {
        return Err(VerifyError::Checksum(format!(
            "sha256 of {} is {actual} but checksums.txt says {expected} for {asset_name}",
            asset.display()
        ))
        .into());
    }
    Ok(())
}

/// Full release verification: signature over the checksums file, then the
/// asset hash. Reads all three files from disk.
pub fn verify_release_asset(
    checksums_path: &Path,
    signature_path: &Path,
    asset: &Path,
    asset_name: &str,
    public_key_b64: &str,
) -> Result<()> {
    let checksums = fs::read(checksums_path)
        .with_context(|| format!("Failed to read {}", checksums_path.display()))?;
    let signature = fs::read_to_string(signature_path)
        .with_context(|| format!("Failed to read {}", signature_path.display()))?;
    verify_signature(&checksums, &signature, public_key_b64)?;
    let checksums = String::from_utf8(checksums)
        .map_err(|_| VerifyError::Checksum("checksums.txt is not UTF-8".to_string()))?;
    verify_sha256(&checksums, asset, asset_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const TEST_PUBLIC_KEY: &str = "RWSD+pKuMle9GDuXt7KwcW0cF6M+NhhAH31FkxTMNh72Ldc2jkOL0XB0";

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/selfhost")
            .join(name)
    }

    #[test]
    fn valid_signature_and_checksum_pass() {
        verify_release_asset(
            &fixture("checksums.txt"),
            &fixture("checksums.txt.minisig"),
            &fixture("a4-linux-x64"),
            "a4-linux-x64",
            TEST_PUBLIC_KEY,
        )
        .unwrap();
        // Every platform's asset is in the fixture.
        for name in [
            "a4-darwin-arm64",
            "a4-darwin-x64",
            "a4-linux-arm64",
            "a4-win32-x64.exe",
        ] {
            verify_release_asset(
                &fixture("checksums.txt"),
                &fixture("checksums.txt.minisig"),
                &fixture(name),
                name,
                TEST_PUBLIC_KEY,
            )
            .unwrap();
        }
    }

    #[test]
    fn wrong_key_signature_is_rejected_and_named() {
        let error = verify_release_asset(
            &fixture("checksums.txt"),
            &fixture("checksums.txt.wrong-key.minisig"),
            &fixture("a4-linux-x64"),
            "a4-linux-x64",
            TEST_PUBLIC_KEY,
        )
        .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<VerifyError>(),
            Some(VerifyError::Signature(_))
        ));
        assert!(
            error.to_string().starts_with("Signature check failed"),
            "{error}"
        );
    }

    #[test]
    fn production_key_rejects_fixture_signature() {
        let error = verify_release_asset(
            &fixture("checksums.txt"),
            &fixture("checksums.txt.minisig"),
            &fixture("a4-linux-x64"),
            "a4-linux-x64",
            super::super::keys::MINISIGN_PUBLIC_KEY,
        )
        .unwrap_err();
        assert!(
            error.to_string().starts_with("Signature check failed"),
            "{error}"
        );
    }

    #[test]
    fn tampered_checksums_fail_signature() {
        let mut checksums = fs::read(fixture("checksums.txt")).unwrap();
        checksums[0] = if checksums[0] == b'0' { b'1' } else { b'0' };
        let signature = fs::read_to_string(fixture("checksums.txt.minisig")).unwrap();
        let error = verify_signature(&checksums, &signature, TEST_PUBLIC_KEY).unwrap_err();
        assert!(
            error.to_string().starts_with("Signature check failed"),
            "{error}"
        );
    }

    #[test]
    fn tampered_asset_fails_checksum_with_detail() {
        let dir = tempfile::tempdir().unwrap();
        let asset = dir.path().join("a4-linux-x64");
        fs::write(&asset, "not the signed binary").unwrap();
        let error = verify_release_asset(
            &fixture("checksums.txt"),
            &fixture("checksums.txt.minisig"),
            &asset,
            "a4-linux-x64",
            TEST_PUBLIC_KEY,
        )
        .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<VerifyError>(),
            Some(VerifyError::Checksum(_))
        ));
        assert!(error.to_string().contains("checksums.txt says"), "{error}");
    }

    #[test]
    fn missing_asset_entry_is_a_checksum_failure() {
        let checksums = fs::read_to_string(fixture("checksums.txt")).unwrap();
        let error = expected_sha256(&checksums, "a4-freebsd-x64").unwrap_err();
        assert_eq!(
            error.to_string(),
            "Checksum check failed: checksums.txt has no entry for a4-freebsd-x64"
        );
    }

    #[test]
    fn parser_tolerates_binary_marker_and_junk() {
        let text = "9b184204d2d540076afbc52c75dd5acd6acff062e4446a83c1672d41837cdf1d *a4-linux-x64\nnot a line\n\nABCD  short\n";
        let parsed = parse_checksums(text);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].1, "a4-linux-x64");
    }

    #[test]
    fn malformed_signature_file_is_named() {
        let error = verify_signature(b"x", "garbage", TEST_PUBLIC_KEY).unwrap_err();
        assert!(error.to_string().contains("malformed"), "{error}");
    }
}
