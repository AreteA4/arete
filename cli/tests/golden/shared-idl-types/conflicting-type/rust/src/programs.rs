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

/// Program SDK for `alpha` (program ID `2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM`).
pub mod alpha {
    use arete_sdk::instruction::{AccountMeta, AccountResolution, ArgField, ArgSchema, ArgType, BuiltInstruction, EnumVariantDef, EnumVariantKind, InstructionError, InstructionHandler};
    use serde::Serialize;

    pub const PROGRAM_ID: &str = "2c35Vf2AKSi7mTvaNdhSrgE3ppGAEyeSSLWNRkxbrQQM";

    /// Content hash of the exact program specification captured at generation time.
    pub const PROGRAM_SPEC_HASH: &str = "arete:h1:program-spec:sha256:07ca57c38922e4521a86f57a079fc28eec26733af78a3bef1af19c69c65b88ab";

    /// Release identity addressing hosted account reads for this program.
    pub const PROGRAM_RELEASE_HASH: &str = "arete:h1:program-release:sha256:7ca2173db2139bb3009579d116fbd6a1bedf01b1885f0b08ecc52cacf8219b7c";

    /// Exact release-addressed read descriptor for this program.
    pub fn read_descriptor() -> arete_sdk::ProgramReadDescriptor {
        arete_sdk::ProgramReadDescriptor::LocalHttp {
            release: arete_sdk::ProgramReleaseReference {
                program_release_hash: PROGRAM_RELEASE_HASH.to_string(),
                program_spec_hash: PROGRAM_SPEC_HASH.to_string(),
            },
        }
    }

    /// Typed params for `configure`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct ConfigureParams {
        pub header: serde_json::Value,
        /// Optional address override for the `authority` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub authority: Option<String>,
        /// Address of the `vault` account.
        pub vault: String,
    }

    /// Builds the `configure` instruction.
    pub fn configure(params: ConfigureParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        configure_handler().build(params)
    }

    /// Raw instruction handler for `configure`.
    pub fn configure_handler() -> InstructionHandler {
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
                ArgSchema { name: "header".to_string(), ty: ArgType::Struct(vec![ArgField { name: "version".to_string(), ty: ArgType::U8 }, ArgField { name: "owner".to_string(), ty: ArgType::Pubkey }]) },
            ],
            errors: vec![],
        }
    }

    /// Program accessor exposed on the stack client's `programs` namespace.
    #[derive(Clone)]
    pub struct AlphaProgram {
        #[allow(dead_code)]
        builder: arete_sdk::ProgramBuilder,
    }

    impl AlphaProgram {
        /// Construct from the connected client's program runtime.
        pub fn from_builder(builder: arete_sdk::ProgramBuilder) -> Self {
            Self { builder }
        }

        pub fn configure(&self, params: ConfigureParams) -> Result<BuiltInstruction, InstructionError> {
            configure(params)
        }
    }
}

/// Program SDK for `beta` (program ID `Br9jAU97qteFboeqv34ph8XTsLnfCPTaZ8NepqqeLzDS`).
pub mod beta {
    use arete_sdk::instruction::{AccountMeta, AccountResolution, ArgField, ArgSchema, ArgType, BuiltInstruction, EnumVariantDef, EnumVariantKind, InstructionError, InstructionHandler};
    use serde::Serialize;

    pub const PROGRAM_ID: &str = "Br9jAU97qteFboeqv34ph8XTsLnfCPTaZ8NepqqeLzDS";

    /// Content hash of the exact program specification captured at generation time.
    pub const PROGRAM_SPEC_HASH: &str = "arete:h1:program-spec:sha256:5c21623087e4efea884fb1f00db5f302c5c4db81673a227ed95d09d42e02e809";

    /// Release identity addressing hosted account reads for this program.
    pub const PROGRAM_RELEASE_HASH: &str = "arete:h1:program-release:sha256:18231d9aaf7d096cbe46a1a171c98a935fe0984deb0301726520b86a4c656beb";

    /// Exact release-addressed read descriptor for this program.
    pub fn read_descriptor() -> arete_sdk::ProgramReadDescriptor {
        arete_sdk::ProgramReadDescriptor::LocalHttp {
            release: arete_sdk::ProgramReleaseReference {
                program_release_hash: PROGRAM_RELEASE_HASH.to_string(),
                program_spec_hash: PROGRAM_SPEC_HASH.to_string(),
            },
        }
    }

    /// Typed params for `configure`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct ConfigureParams {
        pub header: serde_json::Value,
        /// Optional address override for the `authority` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub authority: Option<String>,
        /// Address of the `vault` account.
        pub vault: String,
    }

    /// Builds the `configure` instruction.
    pub fn configure(params: ConfigureParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        configure_handler().build(params)
    }

    /// Raw instruction handler for `configure`.
    pub fn configure_handler() -> InstructionHandler {
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
                ArgSchema { name: "header".to_string(), ty: ArgType::Struct(vec![ArgField { name: "version".to_string(), ty: ArgType::U8 }, ArgField { name: "owner".to_string(), ty: ArgType::Pubkey }, ArgField { name: "flags".to_string(), ty: ArgType::U16 }]) },
            ],
            errors: vec![],
        }
    }

    /// Program accessor exposed on the stack client's `programs` namespace.
    #[derive(Clone)]
    pub struct BetaProgram {
        #[allow(dead_code)]
        builder: arete_sdk::ProgramBuilder,
    }

    impl BetaProgram {
        /// Construct from the connected client's program runtime.
        pub fn from_builder(builder: arete_sdk::ProgramBuilder) -> Self {
            Self { builder }
        }

        pub fn configure(&self, params: ConfigureParams) -> Result<BuiltInstruction, InstructionError> {
            configure(params)
        }
    }
}
