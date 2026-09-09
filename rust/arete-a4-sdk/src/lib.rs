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

pub mod amounts;
mod auth;
pub mod chain;
mod client;
pub mod collation;
mod config;
mod connection;
mod entity;
mod error;
mod frame;
pub mod gateway;
pub mod http;
pub mod instruction;
pub mod operations;
pub mod prelude;
pub mod program;
pub mod program_read_transport;
pub mod read;
pub mod serde_utils;
pub mod session;
pub mod spl;
mod store;
mod stream;
mod subscription;
pub mod transactions;
pub mod view;
pub mod wallet;

pub use amounts::{
    format_raw_to_ui, parse_ui_amount_to_raw, resolve_amount, resolve_amount_to_raw, to_raw_amount,
    AmountError, AmountInput, AmountResolutionInput, ResolvedAmount,
};
pub use auth::{AuthConfig, AuthToken, TokenTransport};
pub use chain::{
    derive_http_endpoint, ChainClient, ChainClock, ChainError, ContextSlotOptions, HttpChainClient,
    MintAccountInfo, NativeBalanceInfo, RawAccountInfo, TokenAccountInfo, TokenBalanceInfo,
    TokenBalanceInput,
};
pub use client::{
    Arete, AreteBuilder, ExecutionResult, OperationInspectionOptions, TransactionOptions, Transport,
};
pub use collation::{collation_key, locale_compare, CollationKey};
pub use config::{AreteConfig, ConnectionConfig};
pub use connection::{ConnectionManager, ConnectionState, SubscriptionLease, SubscriptionOptions};
pub use entity::Stack;
pub use error::{AreteError, AuthErrorCode, SocketIssue};
pub use frame::{
    parse_frame, parse_server_message, parse_snapshot_entities, try_parse_subscribed_frame, Frame,
    Mode, Operation, ProtocolErrorFrame, ServerFrame, ServerMessage, SnapshotEntity, SortConfig,
    SortOrder,
};
pub use gateway::{
    create_hosted_solana_gateway_transports, validate_gateway_binding, HostedSolanaGatewayBindings,
    HostedSolanaGatewayCapabilityBinding, HostedSolanaGatewayTransports, SolanaGatewayAuthMetadata,
};
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
pub use program::{
    AttachedPrograms, ProgramBuilder, ProgramSdk, ProgramStack, Programs, StackWithPrograms,
};
pub use program_read_transport::{
    BearerTokenSource, ProgramReadRequest, ProgramReadTransport, ReadAuthTarget,
};
pub use read::{
    validate_program_read_descriptor, AccountBatchItem, AccountBatchResult, AccountReader,
    ProgramQueryDef, ProgramReadBinding, ProgramReadDescriptor, ProgramReadTransportKind,
    ProgramReleaseReference, QueryExecutor, ReadError, ReadRequestError, StackQueryDef,
};
pub use session::{Session, SessionBuilder, SessionMemberOptions};
pub use store::{deep_merge_with_append, SharedStore, StoreConfig, StoreUpdate};
pub use stream::{
    EntityStream, FilterMapStream, FilteredStream, KeyFilter, MapStream, RichEntityStream,
    RichUpdate, Update, UseStream,
};
pub use transactions::{
    HttpTransactionTransport, TransactionError, TransactionTransport, TransactionTransportError,
};
pub use wallet::{
    ConfirmationLevel, SendOptions, SendResult, TransactionCapabilityError,
    TransactionInspectionOptions, TransactionInspectionResult, TransactionResourceOptions,
    TransactionVersion, WalletAdapter, WalletError, WalletExecutionContext,
};

pub use subscription::{
    ClientMessage, SnapshotOptions, Subscription, SubscriptionQuery, Unsubscription,
    MAX_SUBSCRIPTION_ID_BYTES, PROTOCOL_VERSION,
};
pub use view::{
    RichWatchBuilder, StateView, UseBuilder, ViewBuilder, ViewHandle, Views, WatchBuilder,
};
