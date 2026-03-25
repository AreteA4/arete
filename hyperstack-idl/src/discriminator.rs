//! Discriminator utilities

use sha2::{Digest, Sha256};

pub fn anchor_discriminator(preimage: &str) -> Vec<u8> {
    let hash = Sha256::digest(preimage.as_bytes());
    hash[..8].to_vec()
}

/// Compute an Anchor-compatible discriminator for a given namespace and name.
///
/// The discriminator is the first 8 bytes of SHA256("namespace:name").
/// This is used to uniquely identify instructions and accounts in Anchor programs.
///
/// # Arguments
/// * `namespace` - The namespace (e.g., "global" for instructions, "account" for accounts)
/// * `name` - The name in snake_case for instructions or PascalCase for accounts
///
/// # Returns
/// An 8-byte array containing the discriminator
pub fn compute_discriminator(namespace: &str, name: &str) -> [u8; 8] {
    let preimage = format!("{}:{}", namespace, name);
    let bytes = anchor_discriminator(&preimage);
    let mut result = [0u8; 8];
    result.copy_from_slice(&bytes);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discriminator_global_initialize() {
        // Known Anchor discriminator for "global:initialize"
        let disc = compute_discriminator("global", "initialize");
        // Verify it's 8 bytes and non-zero
        assert_eq!(disc.len(), 8);
        assert!(disc.iter().any(|&b| b != 0));
    }

    #[test]
    fn test_discriminator_consistency() {
        // Same inputs always produce same output
        let disc1 = compute_discriminator("global", "deposit");
        let disc2 = compute_discriminator("global", "deposit");
        assert_eq!(disc1, disc2);
    }

    #[test]
    fn test_discriminator_different_names() {
        // Different names produce different discriminators
        let disc1 = compute_discriminator("global", "deposit");
        let disc2 = compute_discriminator("global", "withdraw");
        assert_ne!(disc1, disc2);
    }
}
