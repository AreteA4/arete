//! Generated program SDK: typed instruction builders grouped per program.
//!
//! Instruction building is pure (no network access). Each program module
//! exposes `PROGRAM_ID`, typed `*Params` structs, `fn <instruction>(params)`
//! builders returning `BuiltInstruction`, raw `*_handler()` accessors, and a
//! `pdas` module with PDA derivation helpers. Programs with a recorded
//! program spec additionally expose `PROGRAM_SPEC_HASH` /
//! `PROGRAM_RELEASE_HASH`, a `read_descriptor()` for release-addressed HTTP
//! reads, and typed `*_accounts()` readers on the program accessor. Standalone
//! output also exports a `ProgramSdk` aggregate for direct/session composition.

/// Program SDK for `vault` (program ID `2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM`).
pub mod vault {
    use arete_sdk::instruction::{AccountMeta, AccountResolution, ArgSchema, ArgType, BuiltInstruction, ErrorMetadata, InstructionError, InstructionHandler};
    use serde::Serialize;

    pub const PROGRAM_ID: &str = "2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM";

    /// Content hash of the exact program specification captured at generation time.
    pub const PROGRAM_SPEC_HASH: &str = "arete:h1:program-spec:sha256:82d33b756cb10907d6585bdc2e1e02170b1286819e317bbe5311de01901b22ba";

    /// Release identity addressing hosted account reads for this program.
    pub const PROGRAM_RELEASE_HASH: &str = "arete:h1:program-release:sha256:472ece19e168367b6bd0a61a8e5908345db325bd1ac3f7d31febdd6a3fe09777";

    /// Exact release-addressed read descriptor for this program.
    pub fn read_descriptor() -> arete_sdk::ProgramReadDescriptor {
        arete_sdk::ProgramReadDescriptor::LocalHttp {
            release: arete_sdk::ProgramReleaseReference {
                program_release_hash: PROGRAM_RELEASE_HASH.to_string(),
                program_spec_hash: PROGRAM_SPEC_HASH.to_string(),
            },
        }
    }

    /// Typed params for `deposit`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct DepositParams {
        pub amount: u64,
        /// Optional address override for the `authority` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub authority: Option<String>,
        /// Address of the `vault` account.
        pub vault: String,
    }

    /// Builds the `deposit` instruction.
    pub fn deposit(params: DepositParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        deposit_handler().build(params)
    }

    /// Raw instruction handler for `deposit`.
    pub fn deposit_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![0],
            accounts: vec![
                AccountMeta {
                    name: "authority".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
                AccountMeta {
                    name: "vault".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
            ],
            args: vec![
                ArgSchema { name: "amount".to_string(), ty: ArgType::U64 },
            ],
            errors: vec![
                ErrorMetadata { code: 0, name: "AmountTooSmall".to_string(), msg: "Amount too small".to_string() },
            ],
        }
    }

    /// Program accessor exposed on the stack client's `programs` namespace.
    #[derive(Clone)]
    pub struct VaultProgram {
        #[allow(dead_code)]
        builder: arete_sdk::ProgramBuilder,
    }

    impl VaultProgram {
        /// Construct from the connected client's program runtime.
        pub fn from_builder(builder: arete_sdk::ProgramBuilder) -> Self {
            Self { builder }
        }

        pub fn deposit(&self, params: DepositParams) -> Result<BuiltInstruction, InstructionError> {
            deposit(params)
        }
    }
}
