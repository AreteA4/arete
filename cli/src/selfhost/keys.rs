//! Release signing keys.
//!
//! `checksums.txt` on every `a4-cli-v<version>` GitHub release is signed with
//! minisign; installers and `a4 self update` verify the signature with this
//! public key before trusting any checksum. The same string is embedded in
//! `docs/public/install.sh`, `docs/public/install.ps1` and
//! `packages/arete/bin/a4.js`; `scripts/check-minisign-pubkey.sh` keeps the
//! four copies identical.
//!
//! Key id `D4FB4D5B83098B6C`. The secret key lives only in the GitHub
//! secrets `A4_MINISIGN_SECRET_KEY` / `A4_MINISIGN_PASSWORD`.

/// minisign public key (base64, as printed by `minisign -G`).
pub const MINISIGN_PUBLIC_KEY: &str = "RWRsiwmDW0371BZbcE1IWD6Y8/KIoAArUAp7mpyG6VweJ5rE3Lf3g5qA";
