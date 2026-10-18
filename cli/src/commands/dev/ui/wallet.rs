//! Running steps with a browser wallet (Wallet Standard) connected to the
//! dev UI.
//!
//! The browser never holds the account's API key: this server prepares and
//! submits on its behalf, and only for requests that pass the
//! guard (the UI's own origin on loopback, see [`super::guard`]).
//!
//! - `POST /dev/v1/wallet/prepare {step, instance?, args, pubkey}` prepares
//!   the step for the connected wallet's public key in the v0 format
//!   wallets take, and returns the preview with each transaction's exact
//!   bytes (`transactionBase64`) for the wallet to sign.
//! - `POST /dev/v1/wallet/submit {executionId, signedTransactions}` takes
//!   the wallet's signed transactions (base64, one per prepared
//!   transaction), checks each is the prepared message with the wallet's
//!   signature in its slot, submits the exact bytes through the Arete API
//!   and waits until the step settles. The wallet signs only
//!   (`signTransaction`); it never sends.
//!
//! **Dev environment only.** Wallets know nothing of a dev environment (a
//! fork), so the UI asks them for the `solana:mainnet` chain. Both routes
//! therefore refuse a transaction whose recent blockhash is not one of the
//! fork's synthetic blockhashes (base58 starting with
//! [`SYNTHETIC_BLOCKHASH_PREFIX`]): such a blockhash never exists on
//! mainnet, so nothing a wallet signs here can land there. Use a dev or
//! burner wallet all the same.
//!
//! **Wallets that change transactions are refused, by design.** Some wallets
//! add instructions (priority fees, compute budget, guards) or otherwise
//! rewrite a transaction before signing. The submit check compares the
//! signed message with the prepared bytes, so such a signature is refused:
//! the runtime only accepts the transaction it prepared and simulated. Use
//! a wallet (or a wallet setting) that signs transactions as given.
//!
//! Browser-signed steps are not recorded in `history.jsonl` (a replay
//! signs with dev wallets); guard rejections are published to the event
//! feed directly, since they leave no intent behind.

use std::sync::Arc;
use std::time::Instant;

use arete_mcp::dev::PreparedStep;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine as _;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;
use serde_json::{json, Value};

use super::run::{
    bad_request, failed, is_pubkey, not_submitted, preview, PendingStep, Signer, StepBody,
};
use super::{error_response, UiState};

/// How a dev environment's synthetic blockhashes start in base58. Steps
/// prepared for a deployment in a dev environment carry one; a mainnet
/// blockhash never does.
pub const SYNTHETIC_BLOCKHASH_PREFIX: &str = "SURFNETxSAFEHASH";

pub fn routes() -> Router<Arc<UiState>> {
    Router::new()
        .route("/dev/v1/wallet/prepare", post(prepare))
        .route("/dev/v1/wallet/submit", post(submit))
}

#[derive(Deserialize)]
struct PrepareBody {
    #[serde(flatten)]
    step: StepBody,
    pubkey: String,
}

fn identity(pubkey: &str) -> Value {
    json!({ "name": "browser wallet", "pubkey": pubkey })
}

async fn prepare(State(state): State<Arc<UiState>>, Json(body): Json<PrepareBody>) -> Response {
    if !is_pubkey(&body.pubkey) {
        return bad_request("pubkey must be the connected wallet's base58 public key");
    }
    let request = match body.step.request("browser".into()) {
        Ok(request) => request,
        Err(response) => return *response,
    };
    let client = session!(state);
    match client.prepare_step(&request, &body.pubkey).await {
        Ok(PreparedStep::Rejected(rejection)) => {
            let result = rejection.result(&request.step, identity(&body.pubkey));
            state.hub.publish(
                "step.failed",
                json!({
                    "executionId": null,
                    "step": request.step,
                    "state": "rejected",
                    "rejected": rejection.code,
                    "error": arete_mcp::dev::rejection_message(&rejection.detail),
                    "hint": rejection.detail["hint"],
                    "instanceId": request.instance,
                    "args": request.args,
                    "wallet": body.pubkey,
                    "source": "dev-ui",
                }),
            );
            Json(result).into_response()
        }
        Ok(PreparedStep::Ready(prepared)) => {
            for (index, transaction) in prepared.transactions.iter().enumerate() {
                if let Err(reason) = refuse_unless_local(transaction) {
                    return error_response(
                        StatusCode::CONFLICT,
                        "dev-ui-not-local",
                        format!(
                            "Refused to hand transaction {index} to the browser wallet: {reason}"
                        ),
                    );
                }
            }
            let response = preview(&prepared, identity(&body.pubkey));
            state.pending.insert(PendingStep {
                request,
                signer: Signer::Browser(body.pubkey),
                prepared: *prepared,
                at: Instant::now(),
            });
            Json(response).into_response()
        }
        Err(error) => failed(error),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitBody {
    execution_id: String,
    signed_transactions: Vec<String>,
}

fn compact_u16(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut value = 0usize;
    for index in 0..3 {
        let byte = *bytes.get(index)?;
        value |= ((byte & 0x7f) as usize) << (7 * index);
        if byte & 0x80 == 0 {
            return Some((value, index + 1));
        }
    }
    None
}

/// Split a wire transaction into its signature slots and message.
fn split(transaction: &[u8]) -> Option<(Vec<[u8; 64]>, &[u8])> {
    let (count, prefix) = compact_u16(transaction)?;
    let message = transaction.get(prefix + 64 * count..)?;
    let slots = (0..count)
        .map(|index| {
            let at = prefix + 64 * index;
            transaction.get(at..at + 64)?.try_into().ok()
        })
        .collect::<Option<Vec<[u8; 64]>>>()?;
    Some((slots, message))
}

/// The account keys a message lists first (the signers come first), and
/// where the recent blockhash after them starts.
fn message_keys_at(message: &[u8]) -> Option<(Vec<[u8; 32]>, usize)> {
    let mut offset = usize::from(message.first()? & 0x80 != 0) + 3;
    let (count, length) = compact_u16(message.get(offset..)?)?;
    offset += length;
    let keys = (0..count)
        .map(|index| {
            let at = offset + 32 * index;
            message.get(at..at + 32)?.try_into().ok()
        })
        .collect::<Option<Vec<[u8; 32]>>>()?;
    Some((keys, offset + 32 * count))
}

fn message_keys(message: &[u8]) -> Option<Vec<[u8; 32]>> {
    message_keys_at(message).map(|(keys, _)| keys)
}

/// A message's recent blockhash.
fn recent_blockhash(message: &[u8]) -> Option<[u8; 32]> {
    let (_, offset) = message_keys_at(message)?;
    message.get(offset..offset + 32)?.try_into().ok()
}

/// Check that `signed` is `prepared` with a valid signature by `signer` in
/// the signer's slot (and every other slot as prepared). Returns why not.
pub fn check_signed(prepared: &[u8], signed: &[u8], signer: &[u8; 32]) -> Result<(), String> {
    let (prepared_slots, prepared_message) =
        split(prepared).ok_or("the prepared transaction is malformed")?;
    let (signed_slots, signed_message) =
        split(signed).ok_or("the signed transaction is malformed")?;
    if prepared_message != signed_message || prepared_slots.len() != signed_slots.len() {
        return Err(
            "the wallet changed the transaction; it must sign the prepared bytes as they are"
                .into(),
        );
    }
    let keys = message_keys(prepared_message).ok_or("the prepared message is malformed")?;
    let index = keys
        .iter()
        .take(prepared_slots.len())
        .position(|key| key == signer)
        .ok_or("the connected wallet is not a signer of this transaction")?;
    for (slot, (before, after)) in prepared_slots.iter().zip(&signed_slots).enumerate() {
        if slot != index && before != after {
            return Err(format!(
                "signature slot {slot} changed; only the wallet's own slot may"
            ));
        }
    }
    let key = VerifyingKey::from_bytes(signer).map_err(|_| "the wallet's public key is invalid")?;
    key.verify_strict(
        prepared_message,
        &Signature::from_bytes(&signed_slots[index]),
    )
    .map_err(|_| "the wallet's signature does not verify".to_string())
}

/// Why `transaction` must not go to a browser wallet, if it must not: its
/// recent blockhash is not one of the fork's synthetic blockhashes, so a
/// wallet could not tell it from a mainnet transaction and it might land
/// there.
pub fn refuse_unless_local(transaction: &[u8]) -> Result<(), String> {
    let (_, message) = split(transaction).ok_or("the prepared transaction is malformed")?;
    let blockhash = recent_blockhash(message).ok_or("the prepared message is malformed")?;
    let encoded = bs58::encode(blockhash).into_string();
    if encoded.starts_with(SYNTHETIC_BLOCKHASH_PREFIX) {
        Ok(())
    } else {
        Err(format!(
            "its recent blockhash {encoded} is not a dev environment's synthetic blockhash \
             ({SYNTHETIC_BLOCKHASH_PREFIX}…), so it could be valid on mainnet. A browser wallet \
             only signs transactions that cannot land outside the dev environment; run the step \
             with a dev wallet"
        ))
    }
}

/// Put `pending` back for another try and answer with `response`.
fn retry(state: &UiState, pending: PendingStep, response: Response) -> Response {
    state.pending.insert(pending);
    response
}

/// Submit the wallet's signatures. The prepared step stays prepared (for
/// another signature) until the Arete API takes it: a malformed or
/// wrong signature, no session yet, or a refused or unreachable submit puts
/// it back.
async fn submit(State(state): State<Arc<UiState>>, Json(body): Json<SubmitBody>) -> Response {
    let Some(pending) = state.pending.take(&body.execution_id, |signer| {
        matches!(signer, Signer::Browser(_))
    }) else {
        return error_response(
            StatusCode::NOT_FOUND,
            "dev-ui-not-prepared",
            "No step prepared for a browser wallet with that execution id is waiting here; prepare it again",
        );
    };
    let Signer::Browser(pubkey) = pending.signer.clone() else {
        return retry(
            &state,
            pending,
            bad_request("This step was prepared for a dev wallet"),
        );
    };
    let signer: [u8; 32] = match bs58::decode(&pubkey)
        .into_vec()
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
    {
        Some(signer) => signer,
        None => return bad_request("The prepared step's wallet is not a public key"),
    };
    if body.signed_transactions.len() != pending.prepared.transactions.len() {
        let expected = pending.prepared.transactions.len();
        return retry(
            &state,
            pending,
            bad_request(format!("Expected {expected} signed transaction(s)")),
        );
    }
    for (index, (encoded, prepared)) in body
        .signed_transactions
        .iter()
        .zip(&pending.prepared.transactions)
        .enumerate()
    {
        if let Err(reason) = refuse_unless_local(prepared) {
            return error_response(
                StatusCode::CONFLICT,
                "dev-ui-not-local",
                format!("Refused to submit transaction {index}: {reason}"),
            );
        }
        let signed = match base64::engine::general_purpose::STANDARD.decode(encoded) {
            Ok(signed) => signed,
            Err(_) => {
                return retry(
                    &state,
                    pending,
                    bad_request(format!("Signed transaction {index} is not base64")),
                )
            }
        };
        if let Err(reason) = check_signed(prepared, &signed, &signer) {
            return retry(
                &state,
                pending,
                bad_request(format!("Signed transaction {index}: {reason}")),
            );
        }
    }
    let client = match super::client(&state) {
        Ok(client) => client,
        Err(response) => return retry(&state, pending, *response),
    };
    match client
        .submit_prepared(
            &pending.prepared,
            &pending.request.step,
            body.signed_transactions,
            identity(&pubkey),
        )
        .await
    {
        Ok(result) => Json(result).into_response(),
        Err(error) if not_submitted(&error) => retry(&state, pending, failed(error)),
        Err(error) => failed(error),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ed25519_dalek::{Signer as _, SigningKey};

    /// A 32-byte blockhash whose base58 starts with the synthetic prefix.
    pub(crate) fn synthetic_blockhash() -> [u8; 32] {
        (16..40)
            .find_map(|pad| {
                let text = format!("{SYNTHETIC_BLOCKHASH_PREFIX}{}", "x".repeat(pad));
                bs58::decode(text).into_vec().ok()?.try_into().ok()
            })
            .unwrap()
    }

    /// A v0 transaction with `signers` as its required signers and no
    /// instructions, on a local (synthetic) blockhash.
    pub(crate) fn transaction(signers: &[&SigningKey]) -> Vec<u8> {
        transaction_on(signers, synthetic_blockhash())
    }

    fn transaction_on(signers: &[&SigningKey], blockhash: [u8; 32]) -> Vec<u8> {
        let mut message = vec![0x80, signers.len() as u8, 0, 0, signers.len() as u8];
        for signer in signers {
            message.extend_from_slice(signer.verifying_key().as_bytes());
        }
        message.extend_from_slice(&blockhash);
        message.extend_from_slice(&[0, 0]);
        let mut tx = vec![signers.len() as u8];
        tx.extend(std::iter::repeat_n(0u8, 64 * signers.len()));
        tx.extend_from_slice(&message);
        tx
    }

    pub(crate) fn sign_slot(tx: &[u8], key: &SigningKey, slot: usize) -> Vec<u8> {
        let (_, message) = split(tx).unwrap();
        let signature = key.sign(message).to_bytes();
        let mut signed = tx.to_vec();
        signed[1 + 64 * slot..1 + 64 * (slot + 1)].copy_from_slice(&signature);
        signed
    }

    #[test]
    fn accepts_the_wallets_signature_on_the_prepared_bytes() {
        let platform = SigningKey::from_bytes(&[1; 32]);
        let wallet = SigningKey::from_bytes(&[2; 32]);
        let prepared = sign_slot(&transaction(&[&platform, &wallet]), &platform, 0);
        let signed = sign_slot(&prepared, &wallet, 1);
        assert_eq!(
            check_signed(&prepared, &signed, wallet.verifying_key().as_bytes()),
            Ok(())
        );
    }

    #[test]
    fn refuses_changed_bytes_foreign_slots_and_bad_signatures() {
        let platform = SigningKey::from_bytes(&[1; 32]);
        let wallet = SigningKey::from_bytes(&[2; 32]);
        let other = SigningKey::from_bytes(&[3; 32]);
        let prepared = sign_slot(&transaction(&[&platform, &wallet]), &platform, 0);
        let key = wallet.verifying_key().to_bytes();

        // A changed message (another blockhash).
        let mut changed = sign_slot(&prepared, &wallet, 1);
        let last = changed.len() - 3;
        changed[last] ^= 1;
        assert!(check_signed(&prepared, &changed, &key)
            .unwrap_err()
            .contains("changed the transaction"));

        // The platform's slot overwritten.
        let mut forged = sign_slot(&prepared, &wallet, 1);
        forged[1] ^= 1;
        assert!(check_signed(&prepared, &forged, &key)
            .unwrap_err()
            .contains("slot 0 changed"));

        // Someone else's signature in the wallet's slot.
        let wrong = sign_slot(&prepared, &other, 1);
        assert!(check_signed(&prepared, &wrong, &key)
            .unwrap_err()
            .contains("does not verify"));

        // Unsigned.
        assert!(check_signed(&prepared, &prepared, &key)
            .unwrap_err()
            .contains("does not verify"));

        // A wallet that is not a signer.
        assert!(
            check_signed(&prepared, &prepared, &other.verifying_key().to_bytes())
                .unwrap_err()
                .contains("not a signer")
        );
        assert!(check_signed(&prepared, &[1, 2], &key).is_err());
    }

    #[test]
    fn only_synthetic_blockhashes_go_to_a_browser_wallet() {
        let wallet = SigningKey::from_bytes(&[2; 32]);
        let local = transaction(&[&wallet]);
        assert_eq!(refuse_unless_local(&local), Ok(()));
        assert!(
            bs58::encode(recent_blockhash(split(&local).unwrap().1).unwrap())
                .into_string()
                .starts_with(SYNTHETIC_BLOCKHASH_PREFIX)
        );

        // A real-looking blockhash could land on mainnet.
        let mainnet = transaction_on(&[&wallet], [7u8; 32]);
        let refused = refuse_unless_local(&mainnet).unwrap_err();
        assert!(
            refused.contains("not a dev environment's synthetic blockhash"),
            "{refused}"
        );
        assert!(refuse_unless_local(&[1, 2, 3]).is_err());
    }
}
