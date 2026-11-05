//! Surface dispatch: call an SDK binding by its surface path.
//!
//! Every program and stack SDK names its bindings by the TypeScript surface
//! path (`rawInstructions.buy`, `instructions.trade.buy`, `addresses.vault`).
//! A [`Dispatch`] maps those paths to Rust calls that take JSON arguments and
//! return a [`DispatchOutput`], so a planner (a server that turns a named
//! action into a transaction for a wallet to sign) can route an action to the
//! binding without knowing its Rust types.
//!
//! Generated program modules export `dispatch::register`, which registers
//! every generated instruction builder under `rawInstructions.<instruction>`
//! (see [`InstructionInput`] for the JSON it reads), and `dispatch::BINDINGS`,
//! the surface path of each with its Rust binding. Hand-written extensions
//! register their own entries with [`Dispatch::entry`].
//!
//! ```rust,ignore
//! let mut actions = arete_sdk::dispatch::Dispatch::new();
//! pump::dispatch::register(&mut actions);
//! let call = Call::new()
//!     .with_input(json!({ "args": { "amount": "1000", "max_sol_cost": "5000" },
//!                         "accounts": { "mint": mint, "user": user } }))
//!     .with_chain(chain);
//! let built = actions.call("rawInstructions.buy", call).await?.into_instruction()?;
//! ```
//!
//! Nothing here opens a connection: the only network input is the
//! [`ChainClient`] a call carries, which this module reaches through the
//! trait alone.

use std::collections::BTreeMap;
use std::fmt::{self, Display};
use std::future::Future;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::chain::ChainClient;
use crate::instruction::{BuildOptions, BuiltAccountMeta, BuiltInstruction, InstructionHandler};
use crate::operations::{describe_prepared_operation, PreparedOperation};
use crate::wallet::WalletAdapter;

/// Why a dispatched call produced no output.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DispatchError {
    /// No entry is registered under the path.
    #[error("no binding is registered for \"{0}\"")]
    UnknownPath(String),
    /// The call's arguments or input do not decode into the binding's types.
    #[error("invalid input for \"{path}\": {message}")]
    Decode {
        /// Where in the call the value lives (`$.args[0]`, `$.input`).
        path: String,
        /// What is wrong with it.
        message: String,
    },
    /// The binding needs something the call does not carry (a chain reader,
    /// a wallet, an argument).
    #[error("{0}")]
    Unavailable(String),
    /// The binding ran and failed; its error text.
    #[error("{0}")]
    Call(String),
}

/// What a dispatched binding returned.
#[derive(Debug, Clone)]
pub enum DispatchOutput {
    /// A plain value (addresses, constants, math, reads) as JSON.
    Value(Value),
    /// A built instruction.
    Instruction(BuiltInstruction),
    /// A prepared operation.
    Prepared(PreparedOperation),
}

impl DispatchOutput {
    /// The output as JSON: a value as is, an instruction as
    /// [`BuiltInstruction::to_artifact`], an operation as
    /// [`describe_prepared_operation`].
    pub fn to_json(&self) -> Value {
        match self {
            Self::Value(value) => value.clone(),
            Self::Instruction(instruction) => instruction.to_artifact(),
            Self::Prepared(operation) => describe_prepared_operation(operation),
        }
    }

    /// The built instruction, or an error naming what the binding returned.
    pub fn into_instruction(self) -> Result<BuiltInstruction, DispatchError> {
        match self {
            Self::Instruction(instruction) => Ok(instruction),
            other => Err(DispatchError::Call(format!(
                "expected an instruction, the binding returned {}",
                other.kind()
            ))),
        }
    }

    /// The prepared operation, or an error naming what the binding returned
    /// (a built instruction does not convert).
    pub fn into_prepared(self) -> Result<PreparedOperation, DispatchError> {
        match self {
            Self::Prepared(operation) => Ok(operation),
            other => Err(DispatchError::Call(format!(
                "expected a prepared operation, the binding returned {}",
                other.kind()
            ))),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::Value(_) => "a value",
            Self::Instruction(_) => "an instruction",
            Self::Prepared(_) => "a prepared operation",
        }
    }
}

/// A dispatched call's result.
pub type DispatchResult = Result<DispatchOutput, DispatchError>;

/// One call: positional arguments and/or an input object, plus the chain
/// reader and wallet the binding may use.
#[derive(Clone, Default)]
pub struct Call {
    args: Vec<Value>,
    input: Option<Value>,
    chain: Option<Arc<dyn ChainClient>>,
    wallet: Option<Arc<dyn WalletAdapter>>,
}

impl fmt::Debug for Call {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Call")
            .field("args", &self.args)
            .field("input", &self.input)
            .field("chain", &self.chain.is_some())
            .field("wallet", &self.wallet.is_some())
            .finish()
    }
}

impl Call {
    /// A call with no arguments, input, chain or wallet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the positional arguments.
    pub fn with_args(mut self, args: Vec<Value>) -> Self {
        self.args = args;
        self
    }

    /// Sets the input object.
    pub fn with_input(mut self, input: Value) -> Self {
        self.input = Some(input);
        self
    }

    /// Sets the chain reader.
    pub fn with_chain(mut self, chain: Arc<dyn ChainClient>) -> Self {
        self.chain = Some(chain);
        self
    }

    /// Sets the wallet.
    pub fn with_wallet(mut self, wallet: Arc<dyn WalletAdapter>) -> Self {
        self.wallet = Some(wallet);
        self
    }

    /// The positional arguments.
    pub fn args(&self) -> &[Value] {
        &self.args
    }

    /// Positional argument `index`, decoded into `T`.
    pub fn arg<T: DeserializeOwned>(&self, index: usize) -> Result<T, DispatchError> {
        let Some(value) = self.args.get(index) else {
            return Err(DispatchError::Unavailable(format!(
                "the call passes {} argument(s); argument {index} is required",
                self.args.len()
            )));
        };
        decode(value.clone(), format!("$.args[{index}]"))
    }

    /// Positional argument `index`, or `None` when the call omits it (a
    /// trailing optional argument).
    pub fn arg_opt<T: DeserializeOwned>(&self, index: usize) -> Result<Option<T>, DispatchError> {
        match self.args.get(index) {
            None => Ok(None),
            Some(value) => decode(value.clone(), format!("$.args[{index}]")).map(Some),
        }
    }

    /// The input object decoded into `T` (`{}` when the call has none).
    pub fn input<T: DeserializeOwned>(&self) -> Result<T, DispatchError> {
        let value = self
            .input
            .clone()
            .unwrap_or_else(|| Value::Object(Map::new()));
        decode(value, "$.input".to_string())
    }

    /// The chain reader, if the call carries one.
    pub fn chain(&self) -> Option<&Arc<dyn ChainClient>> {
        self.chain.as_ref()
    }

    /// The chain reader, or [`DispatchError::Unavailable`].
    pub fn require_chain(&self) -> Result<&Arc<dyn ChainClient>, DispatchError> {
        self.chain.as_ref().ok_or_else(|| {
            DispatchError::Unavailable("the binding reads the chain; the call has no chain".into())
        })
    }

    /// The wallet, if the call carries one.
    pub fn wallet(&self) -> Option<&Arc<dyn WalletAdapter>> {
        self.wallet.as_ref()
    }
}

fn decode<T: DeserializeOwned>(value: Value, path: String) -> Result<T, DispatchError> {
    serde_json::from_value(value).map_err(|error| DispatchError::Decode {
        path,
        message: error.to_string(),
    })
}

type Handler = Arc<dyn Fn(Call) -> BoxFuture<'static, DispatchResult> + Send + Sync>;

/// A map from surface path to binding.
#[derive(Clone, Default)]
pub struct Dispatch {
    entries: BTreeMap<String, Handler>,
    instructions: BTreeMap<String, fn() -> InstructionHandler>,
}

impl fmt::Debug for Dispatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dispatch")
            .field("paths", &self.entries.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Dispatch {
    /// An empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Maps `path` to a binding. A later entry for the same path replaces
    /// the earlier one (an extension overriding a generated binding).
    pub fn entry<F, Fut>(&mut self, path: &str, handler: F) -> &mut Self
    where
        F: Fn(Call) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = DispatchResult> + Send + 'static,
    {
        let handler: Handler = Arc::new(move |call| Box::pin(handler(call)));
        self.entries.insert(path.to_string(), handler);
        self.instructions.remove(path);
        self
    }

    /// Maps `path` to a generated instruction builder: the call's input is an
    /// [`InstructionInput`], the output a [`DispatchOutput::Instruction`].
    /// With a chain on the call, account-field PDA seeds read their
    /// account data from it ([`InstructionHandler::build_with_chain`]).
    pub fn instruction(&mut self, path: &str, handler: fn() -> InstructionHandler) -> &mut Self {
        self.entry(path, move |call: Call| async move {
            let input: InstructionInput = call.input()?;
            let (params, options) = input.into_build()?;
            let handler = handler();
            let built = match call.chain() {
                Some(chain) => handler
                    .build_with_chain(chain.as_ref(), params, &options)
                    .await
                    .map_err(|error| DispatchError::Call(error.to_string()))?,
                None => handler
                    .build_with(params, &options)
                    .map_err(|error| DispatchError::Call(error.to_string()))?,
            };
            Ok(DispatchOutput::Instruction(built))
        });
        self.instructions.insert(path.to_string(), handler);
        self
    }

    /// Adds every entry of `other`, its paths prefixed with `prefix.` (a
    /// stack composing its programs' maps under `programs.<name>`), or as
    /// is when `prefix` is empty.
    pub fn merge(&mut self, prefix: &str, other: Dispatch) -> &mut Self {
        let scoped = |path: String| {
            if prefix.is_empty() {
                path
            } else {
                format!("{prefix}.{path}")
            }
        };
        for (path, handler) in other.entries {
            let path = scoped(path);
            self.instructions.remove(&path);
            self.entries.insert(path, handler);
        }
        for (path, handler) in other.instructions {
            self.instructions.insert(scoped(path), handler);
        }
        self
    }

    /// Whether `path` is registered.
    pub fn contains(&self, path: &str) -> bool {
        self.entries.contains_key(path)
    }

    /// Every registered path, sorted.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// The instruction handler registered under `path` with
    /// [`Dispatch::instruction`], for a caller that builds natively.
    pub fn instruction_handler(&self, path: &str) -> Option<InstructionHandler> {
        self.instructions.get(path).map(|handler| handler())
    }

    /// Calls the binding registered under `path`.
    pub async fn call(&self, path: &str, call: Call) -> DispatchResult {
        let Some(handler) = self.entries.get(path) else {
            return Err(DispatchError::UnknownPath(path.to_string()));
        };
        handler(call).await
    }
}

/// The input of a generated instruction entry: the instruction's args, the
/// addresses of its accounts, and anything the build appends.
///
/// ```json
/// {
///   "args": { "amount": "1000" },
///   "accounts": { "mint": "…", "user": "…" },
///   "remainingAccounts": [{ "address": "…", "writable": true }],
///   "payer": "…",
///   "accountData": { "bonding_curve": "<base64>" }
/// }
/// ```
///
/// `args` and `accounts` merge into the handler's params (an account
/// address wins over its resolution, as everywhere). `accountData` carries
/// the on-chain data that account-field PDA seeds read, base64; without it a
/// call with a chain fetches the data.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstructionInput {
    /// Instruction args (and the helper-only `resolve` map), by name.
    #[serde(default)]
    pub args: Map<String, Value>,
    /// Account addresses, by account name.
    #[serde(default)]
    pub accounts: BTreeMap<String, String>,
    /// Account metas appended after the declared accounts.
    #[serde(default)]
    pub remaining_accounts: Vec<RemainingAccountInput>,
    /// Address that fills wallet-signer accounts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payer: Option<String>,
    /// Raw account data, base64, by account name.
    #[serde(default)]
    pub account_data: BTreeMap<String, String>,
}

/// One remaining account of an [`InstructionInput`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemainingAccountInput {
    /// Base58 address.
    pub address: String,
    /// Whether the account is writable.
    #[serde(default)]
    pub writable: bool,
    /// Whether the account signs.
    #[serde(default)]
    pub signer: bool,
}

impl RemainingAccountInput {
    /// The account meta, once the address is a base58 32-byte public key.
    pub fn to_meta(&self) -> Result<BuiltAccountMeta, DispatchError> {
        let pubkey = self.address.parse().map_err(|_| DispatchError::Decode {
            path: "$.input.remainingAccounts".to_string(),
            message: format!("\"{}\" is not a base58 public key", self.address),
        })?;
        Ok(BuiltAccountMeta {
            pubkey,
            is_signer: self.signer,
            is_writable: self.writable,
        })
    }
}

impl InstructionInput {
    /// The handler params and build options the input names.
    pub fn into_build(self) -> Result<(Value, BuildOptions), DispatchError> {
        let mut params = self.args;
        for (name, address) in self.accounts {
            if params.contains_key(&name) {
                return Err(DispatchError::Decode {
                    path: format!("$.input.accounts.{name}"),
                    message: format!("\"{name}\" is also an arg"),
                });
            }
            params.insert(name, Value::String(address));
        }
        let remaining_accounts = self
            .remaining_accounts
            .iter()
            .map(RemainingAccountInput::to_meta)
            .collect::<Result<_, _>>()?;
        let mut account_data = BTreeMap::new();
        for (name, encoded) in self.account_data {
            use base64::Engine as _;
            let data = base64::engine::general_purpose::STANDARD
                .decode(&encoded)
                .map_err(|error| DispatchError::Decode {
                    path: format!("$.input.accountData.{name}"),
                    message: error.to_string(),
                })?;
            account_data.insert(name, data);
        }
        Ok((
            Value::Object(params),
            BuildOptions {
                payer: self.payer,
                accounts: BTreeMap::new(),
                remaining_accounts,
                account_data,
            },
        ))
    }
}

/// A plain value as a [`DispatchOutput::Value`].
pub fn value<T: Serialize + ?Sized>(value: &T) -> DispatchResult {
    serde_json::to_value(value)
        .map(DispatchOutput::Value)
        .map_err(|error| DispatchError::Call(format!("the result does not serialize: {error}")))
}

/// A fallible binding's value.
pub fn outcome<T: Serialize, E: Display>(result: Result<T, E>) -> DispatchResult {
    match result {
        Ok(result) => value(&result),
        Err(error) => Err(DispatchError::Call(error.to_string())),
    }
}

/// A fallible instruction builder's instruction.
pub fn instruction<E: Display>(result: Result<BuiltInstruction, E>) -> DispatchResult {
    result
        .map(DispatchOutput::Instruction)
        .map_err(|error| DispatchError::Call(error.to_string()))
}

/// A fallible operation's prepared operation.
pub fn prepared<P: Into<PreparedOperation>, E: Display>(result: Result<P, E>) -> DispatchResult {
    result
        .map(|operation| DispatchOutput::Prepared(operation.into()))
        .map_err(|error| DispatchError::Call(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use serde_json::json;

    use super::*;
    use crate::instruction::{AccountMeta, AccountResolution, ArgSchema, ArgType, Pubkey};

    const PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
    const MINT: &str = "So11111111111111111111111111111111111111112";
    const EXTRA: &str = "11111111111111111111111111111111";

    fn transfer_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM.to_string(),
            discriminator: vec![3],
            accounts: vec![AccountMeta {
                name: "mint".to_string(),
                is_signer: false,
                is_writable: true,
                resolution: AccountResolution::UserProvided,
                is_optional: false,
            }],
            args: vec![ArgSchema {
                name: "amount".to_string(),
                ty: ArgType::U64,
            }],
            errors: vec![],
        }
    }

    fn actions() -> Dispatch {
        let mut map = Dispatch::new();
        map.instruction("rawInstructions.transfer", transfer_handler);
        map.entry("math.double", |call: Call| async move {
            let amount: u64 = call.arg(0)?;
            value(&(amount * 2))
        });
        map
    }

    #[tokio::test]
    async fn builds_a_generated_instruction_with_remaining_accounts() {
        let input = json!({
            "args": { "amount": "7" },
            "accounts": { "mint": MINT },
            "remainingAccounts": [{ "address": EXTRA, "writable": true }],
        });
        let built = actions()
            .call("rawInstructions.transfer", Call::new().with_input(input))
            .await
            .unwrap()
            .into_instruction()
            .unwrap();
        assert_eq!(built.data, vec![3, 7, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(built.accounts.len(), 2);
        assert_eq!(built.accounts[0].pubkey, Pubkey::from_str(MINT).unwrap());
        assert_eq!(
            built.accounts[1],
            BuiltAccountMeta {
                pubkey: Pubkey::from_str(EXTRA).unwrap(),
                is_signer: false,
                is_writable: true,
            }
        );
        assert!(actions()
            .instruction_handler("rawInstructions.transfer")
            .is_some());
    }

    #[tokio::test]
    async fn reports_unknown_paths_bad_input_and_failed_builds() {
        let map = actions();
        assert_eq!(
            map.call("rawInstructions.nope", Call::new())
                .await
                .unwrap_err(),
            DispatchError::UnknownPath("rawInstructions.nope".to_string())
        );
        let typo = Call::new().with_input(json!({ "arg": {} }));
        assert!(matches!(
            map.call("rawInstructions.transfer", typo).await.unwrap_err(),
            DispatchError::Decode { path, .. } if path == "$.input"
        ));
        let missing = Call::new().with_input(json!({ "args": { "amount": 1 } }));
        let err = map
            .call("rawInstructions.transfer", missing)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("mint"), "{err}");
    }

    #[tokio::test]
    async fn calls_hand_written_entries_and_merges_under_a_prefix() {
        let mut stack = Dispatch::new();
        stack.merge("programs.token", actions());
        let paths: Vec<&str> = stack.paths().collect();
        assert_eq!(
            paths,
            [
                "programs.token.math.double",
                "programs.token.rawInstructions.transfer"
            ]
        );
        let doubled = stack
            .call(
                "programs.token.math.double",
                Call::new().with_args(vec![json!(21)]),
            )
            .await
            .unwrap();
        assert_eq!(doubled.to_json(), json!(42));
        assert!(stack
            .instruction_handler("programs.token.rawInstructions.transfer")
            .is_some());
    }

    #[test]
    fn an_account_may_not_also_be_an_arg() {
        let input: InstructionInput = serde_json::from_value(json!({
            "args": { "mint": MINT },
            "accounts": { "mint": MINT },
        }))
        .unwrap();
        assert!(matches!(
            input.into_build().unwrap_err(),
            DispatchError::Decode { path, .. } if path == "$.input.accounts.mint"
        ));
    }
}
