//! Optional first-party wallet adapters.
//!
//! The core SDK is RPC-free and ships no adapter: everything here is behind a
//! cargo feature, so the base install's dependency graph is unchanged for
//! users who do not opt in.
//!
//! | Adapter | Feature | Versions |
//! |---|---|---|
//! | [`solana::SolanaWalletAdapter`] | `solana-adapter` | legacy, v0, V1 |

#[cfg(feature = "solana-adapter")]
pub mod solana;
