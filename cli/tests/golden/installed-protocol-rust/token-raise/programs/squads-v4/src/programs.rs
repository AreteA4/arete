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

/// Program SDK for `vote_program` (program ID `Vote111111111111111111111111111111111111111`).
pub mod vote_program {
    use arete_sdk::instruction::{AccountMeta, AccountResolution, BuiltInstruction, InstructionError, InstructionHandler};
    use serde::Serialize;

    pub const PROGRAM_ID: &str = "Vote111111111111111111111111111111111111111";

    /// Content hash of the exact program specification captured at generation time.
    pub const PROGRAM_SPEC_HASH: &str = "arete:h1:program-spec:sha256:79425ade85e0a9614e3f5b9b43e32db349266dc2edd7e7840a4e17b4621b9e86";

    /// Release identity addressing hosted account reads for this program.
    pub const PROGRAM_RELEASE_HASH: &str = "arete:h1:program-release:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    /// Exact release-addressed read descriptor for this program.
    pub fn read_descriptor() -> arete_sdk::ProgramReadDescriptor {
        serde_json::from_str("{\"release\":{\"programReleaseHash\":\"arete:h1:program-release:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\",\"programSpecHash\":\"arete:h1:program-spec:sha256:79425ade85e0a9614e3f5b9b43e32db349266dc2edd7e7840a4e17b4621b9e86\"},\"transport\":{\"binding\":{\"auth\":{\"acceptedKeyClasses\":[\"publishable\",\"secret\"],\"mode\":\"signed_session\",\"required\":true,\"sessionEndpoint\":\"https://api.example.test/ws/sessions\",\"targetId\":\"prb_00000000000000000000000000000077\",\"targetKind\":\"program-read-binding\"},\"endpoint\":\"https://reads.example.test/private/\",\"programReadBindingId\":\"prb_00000000000000000000000000000077\"},\"kind\":\"hosted-binding\"}}").expect("generated hosted program read descriptor must be valid")
    }

    /// Program package release this program SDK was generated from.
    pub const PACKAGE_RELEASE_HASH: &str = "arete:registry-package-release:v2:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    /// Typed params for `ping`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct PingParams {
        /// Address of the `payer` signer.
        #[serde(skip_serializing_if = "Option::is_none")]
        pub payer: Option<String>,
    }

    /// Builds the `ping` instruction.
    pub fn ping(params: PingParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        ping_handler().build(params)
    }

    /// Raw instruction handler for `ping`.
    pub fn ping_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![1, 2, 3, 4, 5, 6, 7, 8],
            accounts: vec![
                AccountMeta {
                    name: "payer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
            ],
            args: vec![],
            errors: vec![],
        }
    }

    /// Program accessor exposed on the stack client's `programs` namespace.
    #[derive(Clone)]
    pub struct VoteProgramProgram {
        builder: arete_sdk::ProgramBuilder,
    }

    impl VoteProgramProgram {
        /// Construct from the connected client's program runtime.
        pub fn from_builder(builder: arete_sdk::ProgramBuilder) -> Self {
            Self { builder }
        }

        /// The context program extension functions take: the client's chain
        /// reader, its wallet, and this accessor.
        pub fn context(&self) -> arete_sdk::ProgramContext<'_, VoteProgramProgram> {
            arete_sdk::ProgramContext::new(self)
        }

        pub fn ping(&self, params: PingParams) -> Result<BuiltInstruction, InstructionError> {
            ping(params)
        }
    }

    impl arete_sdk::ProgramAccessor for VoteProgramProgram {
        fn program_builder(&self) -> &arete_sdk::ProgramBuilder {
            &self.builder
        }
    }
}

pub struct SquadsV4Programs {
    pub vote_program: self::vote_program::VoteProgramProgram,
}

impl arete_sdk::Programs for SquadsV4Programs {
    fn from_builder(builder: arete_sdk::ProgramBuilder) -> Self {
        Self {
            vote_program: self::vote_program::VoteProgramProgram::from_builder(builder),
        }
    }
}

impl arete_sdk::ProgramSdk for SquadsV4Programs {
    fn name() -> &'static str {
        "squads-v4"
    }

    fn gateway() -> Option<arete_sdk::HostedSolanaGatewayBindings> {
        Some(serde_json::from_str("{\"chain\":{\"auth\":{\"acceptedKeyClasses\":[\"anonymous\",\"publishable\",\"secret\"],\"audience\":\"arete:solana-gateway\",\"jwksUrl\":\"https://api.example.test/.well-known/jwks.json\",\"mode\":\"signed_session\",\"required\":true,\"scopes\":[\"read\"],\"sessionEndpoint\":\"https://api.example.test/ws/sessions\",\"targetId\":\"sgb_00000000000000000000000000000001\",\"targetKind\":\"solana-gateway-binding\",\"tokenTransport\":\"bearer\",\"transactionEntitlementRequired\":false},\"authPolicy\":\"signed_session\",\"cluster\":\"mainnet-beta\",\"endpoint\":\"https://solana.example.test/gateway/\",\"region\":\"us-west-1\",\"solanaGatewayBindingId\":\"sgb_00000000000000000000000000000001\"},\"transactions\":{\"auth\":{\"acceptedKeyClasses\":[\"publishable\",\"secret\"],\"audience\":\"arete:solana-gateway\",\"jwksUrl\":\"https://api.example.test/.well-known/jwks.json\",\"mode\":\"signed_session\",\"required\":true,\"scopes\":[\"transaction:inspect\",\"transaction:send\"],\"sessionEndpoint\":\"https://api.example.test/ws/sessions\",\"targetId\":\"sgb_00000000000000000000000000000001\",\"targetKind\":\"solana-gateway-binding\",\"tokenTransport\":\"bearer\",\"transactionEntitlementRequired\":true},\"authPolicy\":\"signed_session\",\"cluster\":\"mainnet-beta\",\"endpoint\":\"https://solana.example.test/gateway/\",\"region\":\"us-west-1\",\"solanaGatewayBindingId\":\"sgb_00000000000000000000000000000001\"}}").expect("generated hosted Solana gateway descriptor must be valid"))
    }

    fn package_release_hash() -> Option<&'static str> {
        Some("arete:registry-package-release:v2:sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc")
    }
}
