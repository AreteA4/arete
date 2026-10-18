//! Arete Protocols: the client for a deployed protocol.
//!
//! A protocol package (`a4 install protocol <ref> --rust`) generates a crate
//! with a [`ProtocolDefinition`] constant and typed publishable and secret
//! clients over [`ProtocolClient`]. The client mints protocol sessions from
//! the app's own key, prepares steps (returning the execution intent and the
//! transactions for the wallet to sign, or a typed rejection), submits the
//! signed transactions, reads instances and participants, updates metadata,
//! follows an instance's event stream, keeps an instance live
//! ([`ProtocolClient::instance_handle`]), and onboards a team's Squads multisig
//! ([`ProtocolClient::squads`]). [`verify_webhook`] checks signed
//! webhook deliveries of the protocol's events, and [`verify_receipt`] and
//! [`verify_receipt_on_chain`] check execution receipts.
//!
//! Mirror of `@usearete/sdk`'s `createProtocolClient`.

mod client;
mod handle;
mod metadata_proof;
mod receipt;
pub mod sse;
mod types;
mod webhooks;

pub use client::{
    protocol_prepare_scope, resolve_protocol_endpoints, ProtocolClient, ProtocolClientOptions,
    ProtocolEndpoints, ProtocolError, ProtocolEventStream, ProtocolEventsOptions,
    ProtocolInstanceListFilter, ProtocolMetadataUpdateOptions, ProtocolPrepareOptions,
    ProtocolSquads, ProtocolWalletActionsRequest, SignedTransaction, ARETE_API_URL_ENV,
    PROTOCOL_PREPARE_ANY_SCOPE, PROTOCOL_READ_SCOPE, PROTOCOL_SUBMIT_SCOPE,
};
pub(crate) use handle::StateViewSource;
pub use handle::{
    ProtocolInstanceHandle, ProtocolInstanceHandleOptions, ProtocolInstanceSnapshot,
    ProtocolInstanceState, ProtocolLiveStream, ProtocolLiveViews, ProtocolStackViews,
};
pub use metadata_proof::{
    canonical_json_sha256, canonicalize_json, protocol_metadata_proof,
    protocol_metadata_proof_message, sign_protocol_metadata_proof, MetadataProofError,
    ProtocolMetadataProofInput, ProtocolMetadataProofMessage, ProtocolMetadataWalletProof,
    SignMetadataProofError, PROTOCOL_METADATA_PROOF_DEFAULT_TTL_SECONDS,
    PROTOCOL_METADATA_PROOF_DOMAIN, PROTOCOL_METADATA_PROOF_MAX_TTL_SECONDS,
};
pub use receipt::{
    receipt_canonical_body, receipt_memo_id, receipt_signing_message, verify_receipt,
    verify_receipt_on_chain, verify_receipt_on_chain_bytes, ProtocolExecutionReceipt,
    ProtocolReceiptDeployment, ProtocolReceiptFees, ProtocolReceiptKey, ProtocolReceiptKeySet,
    ProtocolReceiptSignature, ProtocolReceiptTransaction, ReceiptVerificationError,
    VerifiedReceipt, MEMO_PROGRAM_ID, MEMO_V2_PREFIX, PROTOCOL_EXECUTION_RECEIPT_SCHEMA_V1,
    PROTOCOL_RECEIPT_KEYS_SCHEMA_V1,
};
pub use types::*;
pub use webhooks::{
    sign_webhook, verify_webhook, VerifiedWebhook, VerifyWebhookOptions, WebhookErrorCode,
    WebhookHeaders, WebhookVerificationError, WEBHOOK_ID_HEADER, WEBHOOK_SIGNATURE_HEADER,
    WEBHOOK_TIMESTAMP_HEADER, WEBHOOK_TOLERANCE_SECONDS,
};

#[cfg(test)]
mod tests;
