//! Rust client SDK for connecting to Arete streaming servers.
//!
//! ```rust,ignore
//! use arete_sdk::prelude::*;
//! use ore_stack::{OreStreamStack, DeployParams};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let a4 = Arete::<OreStreamStack>::builder()
//!         .api_key("hspk_…")
//!         .connect()
//!         .await?;
//!
//!     // Views: get, listen (merged entities), watch (Update<T>), watch_rich.
//!     let mut rounds = a4.views.ore_round.latest().watch().take(10);
//!     while let Some(update) = rounds.next().await {
//!         println!("round update: {update:?}");
//!     }
//!
//!     // Program SDK: pure typed instruction builders (mirror of the
//!     // TypeScript `client.programs.<name>.raw.<ix>.build(params)`).
//!     let ix = a4.programs.ore.deploy(DeployParams { /* … */ })?;
//!
//!     Ok(())
//! }
//! ```

// Without `client`, crate-private helpers only the network clients call are
// unreachable; they stay compiled so both configurations share one source.
#![cfg_attr(not(feature = "client"), allow(dead_code))]

/// Version of the extension-authoring surface (`docs/internal/sdk-core-api.md`
/// §9, "Extension API contract"): the program/stack binding traits generated
/// code and extensions implement ([`Programs`], [`ProgramSdk`],
/// [`StackWithPrograms`]), program read attachment, and the instruction
/// helpers generated code imports. Bumped only on a breaking change; recorded
/// as `[package.metadata.arete] extension-api` in this crate's `Cargo.toml`.
/// The TypeScript and Python SDKs export the same value.
pub const EXTENSION_API_VERSION: u32 = 1;

pub mod adapters;
pub mod amounts;
#[cfg(feature = "client")]
mod auth;
pub mod chain;
/// Versioned managed Solana read and discovery contracts.
pub use arete_solana_contracts as managed_solana;
#[cfg(feature = "client")]
mod client;
pub mod collation;
#[cfg(feature = "client")]
mod config;
#[cfg(feature = "client")]
mod connection;
#[cfg(feature = "client")]
#[doc(hidden)]
pub mod credentials;
pub mod dev_environment;
pub mod dispatch;
pub mod encoding;
#[cfg(feature = "client")]
mod entity;
mod error;
mod frame;
#[cfg(feature = "client")]
pub mod gateway;
#[cfg(feature = "client")]
pub mod http;
pub mod instruction;
#[cfg(all(test, feature = "client"))]
mod managed_solana_tests;
pub mod operations;
#[cfg(feature = "client")]
pub mod prelude;
#[cfg(feature = "client")]
pub mod program;
#[cfg(feature = "client")]
pub mod program_read_transport;
#[cfg(feature = "client")]
pub mod protocol;
#[cfg(feature = "client")]
pub mod read;
#[cfg(feature = "client")]
pub mod rpc;
pub mod serde_utils;
#[cfg(feature = "client")]
pub mod session;
pub mod spl;
#[cfg(feature = "client")]
mod store;
#[cfg(feature = "client")]
mod stream;
mod subscription;
pub mod transactions;
#[cfg(feature = "client")]
pub mod view;
pub mod wallet;

#[cfg(feature = "solana-adapter")]
pub use adapters::solana::{
    AdapterTransportSelection, SharedSigner, SolanaAdapterConfig, SolanaOperationSigner,
    SolanaWalletAdapter,
};
pub use amounts::{
    format_raw_to_ui, parse_ui_amount_to_raw, resolve_amount, resolve_amount_to_raw, to_raw_amount,
    AmountError, AmountInput, AmountResolutionInput, ResolvedAmount,
};
#[cfg(feature = "client")]
pub use auth::{
    check_credential_destination, classify_api_key, hosted_websocket_suffixes,
    is_hosted_websocket_host, AccountCredential, ApiKeyClass, AuthConfig, AuthToken, StackRelease,
    TokenTransport, ARETE_API_KEY_ENV, HOSTED_WEBSOCKET_SUFFIX, HOSTED_WEBSOCKET_SUFFIXES_ENV,
};
#[cfg(feature = "client")]
pub use chain::HttpChainClient;
pub use chain::{
    derive_http_endpoint, ChainClient, ChainClock, ChainError, ContextSlotOptions, MintAccountInfo,
    NativeBalanceInfo, RawAccountInfo, TokenAccountInfo, TokenBalanceInfo, TokenBalanceInput,
};
#[cfg(feature = "client")]
pub use client::{
    Arete, AreteBuilder, ExecutionResult, OperationInspectionOptions, TransactionOptions, Transport,
};
pub use collation::{collation_key, locale_compare, CollationKey};
#[cfg(feature = "client")]
pub use config::{AreteConfig, ConnectionConfig};
#[cfg(feature = "client")]
pub use connection::{ConnectionManager, ConnectionState, SubscriptionLease, SubscriptionOptions};
pub use dev_environment::{
    DevEnvironmentError, Environment, EnvironmentOption, SwitchResult, ARETE_DEV_ENV_ENV,
    ARETE_DEV_URL_ENV, ARETE_ENVIRONMENT_ENV,
};
pub use encoding::{decode_base58, encode_base58, keccak256, sha256, Base58Error};
#[cfg(feature = "client")]
pub use entity::Stack;
pub use error::{
    ApiProblemV1, AreteError, AuthErrorCode, GapCode, ReadyRecoveryAction, ReadyRecoveryActionV1,
    RecoveryAction, SocketIssue, StackVersionRefusal, StreamGap, UsageLimit,
    API_PROBLEM_SCHEMA_VERSION, CLAIM_AGENT_MATERIALIZER_PATH, UNTRUSTED_CREDENTIAL_DESTINATION,
};
pub use frame::{
    parse_frame, parse_server_message, parse_snapshot_entities, try_parse_subscribed_frame, Frame,
    Mode, Operation, ProtocolErrorFrame, ReplayWindow, ServerFrame, ServerMessage, SnapshotEntity,
    SortConfig, SortOrder,
};
#[cfg(feature = "client")]
pub use gateway::{
    create_hosted_solana_gateway_transports, validate_gateway_binding, HostedSolanaGatewayBindings,
    HostedSolanaGatewayCapabilityBinding, HostedSolanaGatewayTransports, SolanaGatewayAuthMetadata,
};
#[cfg(feature = "client")]
pub use http::{
    fetch_json, AuthTokenRequest, AuthTokenTarget, AuthedRequest, AuthedResponse, HttpAuthClient,
    HttpMethod, TokenSource, TokenTargetKind,
};
pub use instruction::{
    AccountMeta, AccountResolution, ArgField, ArgSchema, ArgType, BuildOptions, BuiltAccountMeta,
    BuiltInstruction, EnumVariantDef, EnumVariantKind, ErrorMetadata, InstructionError,
    InstructionHandler, PdaConfig, PdaSeed, Pubkey,
};
pub use operations::{
    append_flow_transactions, append_transaction_instructions, create_prepared_flow,
    create_prepared_instruction, create_prepared_transaction, create_prepared_transaction_body,
    describe_prepared_operation, execute_prepared_operation, format_program_error,
    inspect_prepared_operation, parse_program_error, prepend_flow_transaction_instructions,
    prepend_transaction_instructions, ExecuteOptions, ExecutionHost, FailurePhase,
    OperationCallback, OperationError, OperationExecutionError, OperationExecutionEvent,
    OperationInspection, OperationInspectionError, OperationKind, OperationReceipt,
    OperationTransactionReceipt, PreparedFlow, PreparedInstruction, PreparedOperation,
    PreparedTransaction, PreparedTransactionBody, PreparedTransactionChildren,
    PreparedTransactionInstruction, ProgramError, Signer, SignerRegistry,
    TransactionFailureOutcome, TransactionOutcome,
};
#[cfg(feature = "client")]
pub use program::{
    same_program, AttachedPrograms, ProgramAccessor, ProgramBuilder, ProgramContext, ProgramSdk,
    ProgramStack, Programs, StackWithPrograms,
};
#[cfg(feature = "client")]
pub use program_read_transport::{
    BearerTokenSource, ProgramReadRequest, ProgramReadTransport, ReadAuthTarget,
};
#[cfg(feature = "client")]
pub use read::{
    validate_program_read_descriptor, AccountBatchItem, AccountBatchResult, AccountReader,
    ProgramQueryDef, ProgramReadBinding, ProgramReadDescriptor, ProgramReadTransportKind,
    ProgramReleaseReference, QueryExecutor, ReadError, ReadRequestError, StackQueryDef,
};
#[cfg(feature = "client")]
pub use rpc::RpcTransactionTransport;
#[cfg(feature = "client")]
pub use session::{Session, SessionBuilder, SessionMemberOptions};
#[cfg(feature = "client")]
pub use store::{deep_merge_with_append, SharedStore, StoreConfig, StoreEvent, StoreUpdate};
#[cfg(feature = "client")]
pub use stream::{
    EntityStream, FilterMapStream, FilteredStream, KeyFilter, MapStream, RichEntityStream,
    RichUpdate, Update, UseStream,
};
#[cfg(feature = "client")]
pub use transactions::HttpTransactionTransport;
pub use transactions::{TransactionError, TransactionTransport, TransactionTransportError};
pub use wallet::{
    ConfirmationLevel, SendOptions, SendResult, TransactionCapabilityError,
    TransactionInspectionOptions, TransactionInspectionResult, TransactionResourceOptions,
    TransactionVersion, WalletAdapter, WalletError, WalletExecutionContext,
};

pub use subscription::{
    ClientMessage, SnapshotOptions, Subscription, SubscriptionQuery, Unsubscription,
    MAX_SUBSCRIPTION_ID_BYTES, PROTOCOL_VERSION,
};
#[cfg(feature = "client")]
pub use view::{
    GetOptions, RichWatchBuilder, StateView, UseBuilder, ViewBuilder, ViewError, ViewHandle, Views,
    WatchBuilder,
};

#[cfg(test)]
mod tests {
    /// The crate records the extension API it exports, so tools can read it
    /// from the manifest without compiling anything.
    #[test]
    fn extension_api_version_is_recorded_in_the_manifest() {
        let manifest = include_str!("../Cargo.toml");
        let section = manifest
            .split("[package.metadata.arete]")
            .nth(1)
            .expect("Cargo.toml has [package.metadata.arete]");
        let recorded = section
            .lines()
            .find_map(|line| line.trim().strip_prefix("extension-api"))
            .and_then(|rest| rest.trim().strip_prefix('='))
            .map(|value| value.trim().parse::<u32>().expect("an integer"))
            .expect("extension-api is recorded");
        assert_eq!(recorded, super::EXTENSION_API_VERSION);
    }
}
