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
    pub const PROGRAM_SPEC_HASH: &str = "arete:h1:program-spec:sha256:5469c29441aaeac7da96e8f1ac69a42de33bf7805a76c83c75da9d762213b952";

    /// Release identity addressing hosted account reads for this program.
    pub const PROGRAM_RELEASE_HASH: &str = "arete:h1:program-release:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    /// Exact release-addressed read descriptor for this program.
    pub fn read_descriptor() -> arete_sdk::ProgramReadDescriptor {
        serde_json::from_str("{\"release\":{\"programReleaseHash\":\"arete:h1:program-release:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\",\"programSpecHash\":\"arete:h1:program-spec:sha256:5469c29441aaeac7da96e8f1ac69a42de33bf7805a76c83c75da9d762213b952\"},\"transport\":{\"binding\":{\"auth\":{\"mode\":\"signed_session\",\"required\":true,\"sessionEndpoint\":\"https://api.example.test/ws/sessions\",\"targetId\":\"prb_00000000000000000000000000000001\",\"targetKind\":\"program-read-binding\"},\"endpoint\":\"https://reads.example.test/vault/\",\"programReadBindingId\":\"prb_00000000000000000000000000000001\"},\"kind\":\"hosted-binding\"}}").expect("generated hosted program read descriptor must be valid")
    }

    /// Program package release this program SDK was generated from.
    pub const PACKAGE_RELEASE_HASH: &str = "arete:registry-package-release:v2:sha256:7777777777777777777777777777777777777777777777777777777777777777";

    /// Typed params for `deposit`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct DepositParams {
        pub amount: u64,
        /// Address of the `authority` signer.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub authority: Option<String>,
        /// Address of the `vault` account.
        pub vault: String,
        /// Address of the `mint` account.
        pub mint: String,
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
                AccountMeta {
                    name: "mint".to_string(),
                    is_signer: false,
                    is_writable: false,
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
        builder: arete_sdk::ProgramBuilder,
    }

    impl VaultProgram {
        /// Construct from the connected client's program runtime.
        pub fn from_builder(builder: arete_sdk::ProgramBuilder) -> Self {
            Self { builder }
        }

        /// The context program extension functions take: the client's chain
        /// reader, its wallet, and this accessor.
        pub fn context(&self) -> arete_sdk::ProgramContext<'_, VaultProgram> {
            arete_sdk::ProgramContext::new(self)
        }

        pub fn deposit(&self, params: DepositParams) -> Result<BuiltInstruction, InstructionError> {
            deposit(params)
        }

        /// Typed reader for `Vault` accounts (release-addressed HTTP reads).
        pub fn vault_accounts(&self) -> Result<arete_sdk::AccountReader<crate::types::Vault>, arete_sdk::AreteError> {
            Ok(arete_sdk::AccountReader::new(
                "Vault",
                std::sync::Arc::new(self.builder.account_transport("vault", &read_descriptor())?),
            ))
        }
    }

    impl arete_sdk::ProgramAccessor for VaultProgram {
        fn program_builder(&self) -> &arete_sdk::ProgramBuilder {
            &self.builder
        }
    }
}

pub struct VaultPrograms {
    pub vault: self::vault::VaultProgram,
}

impl arete_sdk::Programs for VaultPrograms {
    fn from_builder(builder: arete_sdk::ProgramBuilder) -> Self {
        Self {
            vault: self::vault::VaultProgram::from_builder(builder),
        }
    }
}

impl arete_sdk::ProgramSdk for VaultPrograms {
    fn name() -> &'static str {
        "vault"
    }

    fn gateway() -> Option<arete_sdk::HostedSolanaGatewayBindings> {
        Some(serde_json::from_str("{\"chain\":{\"auth\":{\"acceptedKeyClasses\":[\"publishable\",\"secret\"],\"audience\":\"arete:solana-gateway\",\"jwksUrl\":\"https://api.example.test/.well-known/jwks.json\",\"mode\":\"signed_session\",\"required\":true,\"scopes\":[\"read\"],\"sessionEndpoint\":\"https://api.example.test/ws/sessions\",\"targetId\":\"sgb_00000000000000000000000000000001\",\"targetKind\":\"solana-gateway-binding\",\"tokenTransport\":\"bearer\",\"transactionEntitlementRequired\":false},\"authPolicy\":\"signed_session\",\"cluster\":\"mainnet-beta\",\"endpoint\":\"https://solana.example.test/gateway/\",\"region\":\"us-west-1\",\"solanaGatewayBindingId\":\"sgb_00000000000000000000000000000001\"},\"transactions\":{\"auth\":{\"acceptedKeyClasses\":[\"publishable\",\"secret\"],\"audience\":\"arete:solana-gateway\",\"jwksUrl\":\"https://api.example.test/.well-known/jwks.json\",\"mode\":\"signed_session\",\"required\":true,\"scopes\":[\"transaction:inspect\",\"transaction:send\"],\"sessionEndpoint\":\"https://api.example.test/ws/sessions\",\"targetId\":\"sgb_00000000000000000000000000000001\",\"targetKind\":\"solana-gateway-binding\",\"tokenTransport\":\"bearer\",\"transactionEntitlementRequired\":true},\"authPolicy\":\"signed_session\",\"cluster\":\"mainnet-beta\",\"endpoint\":\"https://solana.example.test/gateway/\",\"region\":\"us-west-1\",\"solanaGatewayBindingId\":\"sgb_00000000000000000000000000000001\"}}").expect("generated hosted Solana gateway descriptor must be valid"))
    }

    fn package_release_hash() -> Option<&'static str> {
        Some("arete:registry-package-release:v2:sha256:7777777777777777777777777777777777777777777777777777777777777777")
    }
}
