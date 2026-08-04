//! Generated program SDK: typed instruction builders grouped per program.
//!
//! Instruction building is pure (no network access). Each program module
//! exposes `PROGRAM_ID`, typed `*Params` structs, `fn <instruction>(params)`
//! builders returning `BuiltInstruction`, raw `*_handler()` accessors, and a
//! `pdas` module with PDA derivation helpers. Programs with a recorded
//! program spec additionally expose `PROGRAM_SPEC_HASH` /
//! `PROGRAM_RELEASE_HASH`, a `read_descriptor()` for release-addressed HTTP
//! reads, and typed `*_accounts()` readers on the program accessor.

/// Program SDK for `ore` (program ID `oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv`).
pub mod ore {
    use arete_sdk::instruction::{AccountMeta, AccountResolution, ArgSchema, ArgType, BuiltInstruction, ErrorMetadata, InstructionError, InstructionHandler, PdaConfig, PdaSeed};
    use serde::Serialize;

    pub const PROGRAM_ID: &str = "oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv";

    /// Content hash of the exact program specification captured at generation time.
    pub const PROGRAM_SPEC_HASH: &str = "arete:h1:program-spec:sha256:b37beb6e8df0a55316f39cea21f3f3f5bc827aa7e6a54291398edbf1db58ff6b";

    /// Release identity addressing hosted account reads for this program.
    pub const PROGRAM_RELEASE_HASH: &str = "arete:h1:program-release:sha256:adff79886735a8cacfb69bd3a5371f5cfeec18d099a289cce4755b9104ffd986";

    /// Release-addressed read descriptor for this program (HTTP reads over
    /// the client's HTTP base URL).
    pub fn read_descriptor() -> arete_sdk::ProgramReadDescriptor {
        arete_sdk::ProgramReadDescriptor::LocalHttp {
            release: arete_sdk::ProgramReleaseReference {
                program_release_hash: PROGRAM_RELEASE_HASH.to_string(),
                program_spec_hash: PROGRAM_SPEC_HASH.to_string(),
            },
        }
    }

    /// Typed params for `automate`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct AutomateParams {
        pub amount: u64,
        pub deposit: u64,
        pub fee: u64,
        pub mask: u64,
        pub strategy: u8,
        pub reload: u64,
        /// Optional address override for the `signer` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub signer: Option<String>,
        /// Address of the `automation` account.
        pub automation: String,
        /// Address of the `executor` account.
        pub executor: String,
        /// Address of the `miner` account.
        pub miner: String,
    }

    /// Configures or closes a miner automation account.
    /// Automation PDA seeds: ["automation", signer].
    /// Miner PDA seeds: ["miner", signer].
    ///
    /// Codegen notes:
    /// - account `automation` degraded to user-provided (PDA 'automation': seed references account 'authority' not present in this instruction)
    /// - account `miner` degraded to user-provided (PDA 'miner': seed references account 'authority' not present in this instruction)
    pub fn automate(params: AutomateParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        automate_handler().build(params)
    }

    /// Raw instruction handler for `automate`.
    pub fn automate_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![0],
            accounts: vec![
                AccountMeta {
                    name: "signer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
                // [arete codegen] account `automation` degraded to user-provided (PDA 'automation': seed references account 'authority' not present in this instruction)
                AccountMeta {
                    name: "automation".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "executor".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                // [arete codegen] account `miner` degraded to user-provided (PDA 'miner': seed references account 'authority' not present in this instruction)
                AccountMeta {
                    name: "miner".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "systemProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("11111111111111111111111111111111".to_string()),
                    is_optional: false,
                },
            ],
            args: vec![
                ArgSchema { name: "amount".to_string(), ty: ArgType::U64 },
                ArgSchema { name: "deposit".to_string(), ty: ArgType::U64 },
                ArgSchema { name: "fee".to_string(), ty: ArgType::U64 },
                ArgSchema { name: "mask".to_string(), ty: ArgType::U64 },
                ArgSchema { name: "strategy".to_string(), ty: ArgType::U8 },
                ArgSchema { name: "reload".to_string(), ty: ArgType::U64 },
            ],
            errors: vec![
                ErrorMetadata { code: 0, name: "AmountTooSmall".to_string(), msg: "Amount too small".to_string() },
                ErrorMetadata { code: 1, name: "NotAuthorized".to_string(), msg: "Not authorized".to_string() },
                ErrorMetadata { code: 2, name: "InvalidExecutor".to_string(), msg: "Invalid executor".to_string() },
            ],
        }
    }

    /// Typed params for `checkpoint`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct CheckpointParams {
        /// Optional address override for the `signer` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub signer: Option<String>,
        /// Address of the `authority` account.
        pub authority: String,
        /// Address of the `round` account.
        pub round: String,
    }

    /// Settles miner rewards for a completed round.
    /// Treasury PDA seeds: ["treasury"].
    pub fn checkpoint(params: CheckpointParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        checkpoint_handler().build(params)
    }

    /// Raw instruction handler for `checkpoint`.
    pub fn checkpoint_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![2],
            accounts: vec![
                AccountMeta {
                    name: "signer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
                AccountMeta {
                    name: "authority".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "automation".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("automation".to_string()), PdaSeed::AccountRef("authority".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "board".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("board".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "miner".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("miner".to_string()), PdaSeed::AccountRef("authority".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "round".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "treasury".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("treasury".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "systemProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("11111111111111111111111111111111".to_string()),
                    is_optional: false,
                },
            ],
            args: vec![],
            errors: vec![
                ErrorMetadata { code: 0, name: "AmountTooSmall".to_string(), msg: "Amount too small".to_string() },
                ErrorMetadata { code: 1, name: "NotAuthorized".to_string(), msg: "Not authorized".to_string() },
                ErrorMetadata { code: 2, name: "InvalidExecutor".to_string(), msg: "Invalid executor".to_string() },
            ],
        }
    }

    /// Typed params for `claimSol`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct ClaimSolParams {
        /// Optional address override for the `signer` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub signer: Option<String>,
        /// Address of the `miner` account.
        pub miner: String,
    }

    /// Claims SOL rewards from the miner account.
    ///
    /// Codegen notes:
    /// - account `miner` degraded to user-provided (PDA 'miner': seed references account 'authority' not present in this instruction)
    pub fn claim_sol(params: ClaimSolParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        claim_sol_handler().build(params)
    }

    /// Raw instruction handler for `claimSol`.
    pub fn claim_sol_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![3],
            accounts: vec![
                AccountMeta {
                    name: "signer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
                AccountMeta {
                    name: "board".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("board".to_string())] }),
                    is_optional: false,
                },
                // [arete codegen] account `miner` degraded to user-provided (PDA 'miner': seed references account 'authority' not present in this instruction)
                AccountMeta {
                    name: "miner".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "systemProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("11111111111111111111111111111111".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "oreProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv".to_string()),
                    is_optional: false,
                },
            ],
            args: vec![],
            errors: vec![
                ErrorMetadata { code: 0, name: "AmountTooSmall".to_string(), msg: "Amount too small".to_string() },
                ErrorMetadata { code: 1, name: "NotAuthorized".to_string(), msg: "Not authorized".to_string() },
                ErrorMetadata { code: 2, name: "InvalidExecutor".to_string(), msg: "Invalid executor".to_string() },
            ],
        }
    }

    /// Typed params for `claimOre`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct ClaimOreParams {
        pub bps: u64,
        /// Optional address override for the `signer` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub signer: Option<String>,
        /// Address of the `miner` account.
        pub miner: String,
        /// Address of the `recipient` account.
        pub recipient: String,
        /// Address of the `treasuryTokens` account.
        #[serde(rename = "treasuryTokens")]
        pub treasury_tokens: String,
    }

    /// Claims a percentage of ORE token rewards from the treasury vault.
    /// The current instruction encodes bps as u64. Legacy empty payloads are accepted by the program as 10000 bps.
    ///
    /// Codegen notes:
    /// - account `miner` degraded to user-provided (PDA 'miner': seed references account 'authority' not present in this instruction)
    pub fn claim_ore(params: ClaimOreParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        claim_ore_handler().build(params)
    }

    /// Raw instruction handler for `claimOre`.
    pub fn claim_ore_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![4],
            accounts: vec![
                AccountMeta {
                    name: "signer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
                AccountMeta {
                    name: "board".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("board".to_string())] }),
                    is_optional: false,
                },
                // [arete codegen] account `miner` degraded to user-provided (PDA 'miner': seed references account 'authority' not present in this instruction)
                AccountMeta {
                    name: "miner".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "mint".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Known("oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "recipient".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "treasury".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("treasury".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "treasuryTokens".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "systemProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("11111111111111111111111111111111".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "tokenProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "associatedTokenProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "oreProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv".to_string()),
                    is_optional: false,
                },
            ],
            args: vec![
                ArgSchema { name: "bps".to_string(), ty: ArgType::U64 },
            ],
            errors: vec![
                ErrorMetadata { code: 0, name: "AmountTooSmall".to_string(), msg: "Amount too small".to_string() },
                ErrorMetadata { code: 1, name: "NotAuthorized".to_string(), msg: "Not authorized".to_string() },
                ErrorMetadata { code: 2, name: "InvalidExecutor".to_string(), msg: "Invalid executor".to_string() },
            ],
        }
    }

    /// Typed params for `close`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct CloseParams {
        /// Optional address override for the `signer` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub signer: Option<String>,
        /// Address of the `rentPayer` account.
        #[serde(rename = "rentPayer")]
        pub rent_payer: String,
        /// Address of the `round` account.
        pub round: String,
    }

    /// Closes an expired round account and returns rent to the payer.
    /// Round PDA seeds: ["round", round_id].
    /// Treasury PDA seeds: ["treasury"].
    pub fn close(params: CloseParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        close_handler().build(params)
    }

    /// Raw instruction handler for `close`.
    pub fn close_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![5],
            accounts: vec![
                AccountMeta {
                    name: "signer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
                AccountMeta {
                    name: "board".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("board".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "rentPayer".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "round".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "treasury".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("treasury".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "systemProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("11111111111111111111111111111111".to_string()),
                    is_optional: false,
                },
            ],
            args: vec![],
            errors: vec![
                ErrorMetadata { code: 0, name: "AmountTooSmall".to_string(), msg: "Amount too small".to_string() },
                ErrorMetadata { code: 1, name: "NotAuthorized".to_string(), msg: "Not authorized".to_string() },
                ErrorMetadata { code: 2, name: "InvalidExecutor".to_string(), msg: "Invalid executor".to_string() },
            ],
        }
    }

    /// Typed params for `deploy`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct DeployParams {
        pub amount: u64,
        pub squares: u32,
        /// Optional address override for the `signer` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub signer: Option<String>,
        /// Address of the `authority` account.
        pub authority: String,
        /// Address of the `round` account.
        pub round: String,
        /// Optional address of the `entropyVar` account.
        #[serde(rename = "entropyVar")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub entropy_var: Option<String>,
        /// Optional address of the `entropyProgram` account.
        #[serde(rename = "entropyProgram")]
        #[serde(skip_serializing_if = "Option::is_none")]
        pub entropy_program: Option<String>,
    }

    /// Deploys SOL to selected squares for the current round.
    /// Automation PDA seeds: ["automation", authority].
    /// Config PDA seeds: ["config"].
    /// Miner PDA seeds: ["miner", authority].
    /// Round PDA seeds: ["round", board.round_id].
    pub fn deploy(params: DeployParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        deploy_handler().build(params)
    }

    /// Raw instruction handler for `deploy`.
    pub fn deploy_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![6],
            accounts: vec![
                AccountMeta {
                    name: "signer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
                AccountMeta {
                    name: "authority".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "automation".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("automation".to_string()), PdaSeed::AccountRef("authority".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "board".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("board".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "config".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("config".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "miner".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("miner".to_string()), PdaSeed::AccountRef("authority".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "round".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "treasury".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("treasury".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "systemProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("11111111111111111111111111111111".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "oreProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "entropyVar".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: true,
                },
                AccountMeta {
                    name: "entropyProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::UserProvided,
                    is_optional: true,
                },
            ],
            args: vec![
                ArgSchema { name: "amount".to_string(), ty: ArgType::U64 },
                ArgSchema { name: "squares".to_string(), ty: ArgType::U32 },
            ],
            errors: vec![
                ErrorMetadata { code: 0, name: "AmountTooSmall".to_string(), msg: "Amount too small".to_string() },
                ErrorMetadata { code: 1, name: "NotAuthorized".to_string(), msg: "Not authorized".to_string() },
                ErrorMetadata { code: 2, name: "InvalidExecutor".to_string(), msg: "Invalid executor".to_string() },
            ],
        }
    }

    /// Typed params for `log`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct LogParams {
        /// Optional address override for the `signer` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub signer: Option<String>,
    }

    /// Emits an arbitrary log message from the board PDA.
    /// Bytes following the discriminator are logged verbatim.
    pub fn log(params: LogParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        log_handler().build(params)
    }

    /// Raw instruction handler for `log`.
    pub fn log_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![8],
            accounts: vec![
                AccountMeta {
                    name: "signer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
            ],
            args: vec![],
            errors: vec![
                ErrorMetadata { code: 0, name: "AmountTooSmall".to_string(), msg: "Amount too small".to_string() },
                ErrorMetadata { code: 1, name: "NotAuthorized".to_string(), msg: "Not authorized".to_string() },
                ErrorMetadata { code: 2, name: "InvalidExecutor".to_string(), msg: "Invalid executor".to_string() },
            ],
        }
    }

    /// Typed params for `reset`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct ResetParams {
        /// Optional address override for the `signer` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub signer: Option<String>,
        /// Address of the `feeCollector` account.
        #[serde(rename = "feeCollector")]
        pub fee_collector: String,
        /// Address of the `round` account.
        pub round: String,
        /// Address of the `roundNext` account.
        #[serde(rename = "roundNext")]
        pub round_next: String,
        /// Address of the `topMiner` account.
        #[serde(rename = "topMiner")]
        pub top_miner: String,
        /// Address of the `treasuryTokens` account.
        #[serde(rename = "treasuryTokens")]
        pub treasury_tokens: String,
        /// Address of the `entropyVar` account.
        #[serde(rename = "entropyVar")]
        pub entropy_var: String,
        /// Address of the `mintAuthority` account.
        #[serde(rename = "mintAuthority")]
        pub mint_authority: String,
    }

    /// Finalizes the current round, mints rewards, and opens the next round.
    /// Board PDA seeds: ["board"].
    /// Treasury PDA seeds: ["treasury"].
    /// Round PDA seeds: ["round", board.round_id] and ["round", board.round_id + 1].
    pub fn reset(params: ResetParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        reset_handler().build(params)
    }

    /// Raw instruction handler for `reset`.
    pub fn reset_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![9],
            accounts: vec![
                AccountMeta {
                    name: "signer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
                AccountMeta {
                    name: "board".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("board".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "config".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("config".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "feeCollector".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "mint".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Known("oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "round".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "roundNext".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "topMiner".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "treasury".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("treasury".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "treasuryTokens".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "systemProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("11111111111111111111111111111111".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "tokenProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "oreProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "slotHashesSysvar".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("SysvarS1otHashes111111111111111111111111111".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "entropyVar".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "entropyProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "mintAuthority".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "mintProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("mintzxW6Kckmeyh1h6Zfdj9QcYgCzhPSGiC8ChZ6fCx".to_string()),
                    is_optional: false,
                },
            ],
            args: vec![],
            errors: vec![
                ErrorMetadata { code: 0, name: "AmountTooSmall".to_string(), msg: "Amount too small".to_string() },
                ErrorMetadata { code: 1, name: "NotAuthorized".to_string(), msg: "Not authorized".to_string() },
                ErrorMetadata { code: 2, name: "InvalidExecutor".to_string(), msg: "Invalid executor".to_string() },
            ],
        }
    }

    /// Typed params for `buyback`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct BuybackParams {
        /// Address of the `managerSol` account.
        #[serde(rename = "managerSol")]
        pub manager_sol: String,
        /// Address of the `treasuryOre` account.
        #[serde(rename = "treasuryOre")]
        pub treasury_ore: String,
        /// Address of the `treasurySol` account.
        #[serde(rename = "treasurySol")]
        pub treasury_sol: String,
        /// Address of the `stakeTreasury` account.
        #[serde(rename = "stakeTreasury")]
        pub stake_treasury: String,
        /// Address of the `stakeTreasuryOre` account.
        #[serde(rename = "stakeTreasuryOre")]
        pub stake_treasury_ore: String,
        /// Address of the `stakeVesting` account.
        #[serde(rename = "stakeVesting")]
        pub stake_vesting: String,
        /// Address of the `oreStakeProgram` account.
        #[serde(rename = "oreStakeProgram")]
        pub ore_stake_program: String,
    }

    /// Swaps vaulted SOL to ORE through Jupiter, distributes staking yield, and burns the remainder.
    /// The 15 declared accounts are followed by Jupiter route accounts, and raw Jupiter instruction data follows the discriminator.
    pub fn buyback(params: BuybackParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        buyback_handler().build(params)
    }

    /// Raw instruction handler for `buyback`.
    pub fn buyback_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![13],
            accounts: vec![
                AccountMeta {
                    name: "signer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Known("HNWhK5f8RMWBqcA7mXJPaxdTPGrha3rrqUrri7HSKb3T".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "board".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("board".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "config".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("config".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "manager".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Known("DJqfQWB8tZE6fzqWa8okncDh7ciTuD8QQKp1ssNETWee".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "managerSol".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "mint".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Known("oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "treasury".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("treasury".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "treasuryOre".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "treasurySol".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "stakeTreasury".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "stakeTreasuryOre".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "stakeVesting".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "tokenProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "oreProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "oreStakeProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
            ],
            args: vec![],
            errors: vec![
                ErrorMetadata { code: 0, name: "AmountTooSmall".to_string(), msg: "Amount too small".to_string() },
                ErrorMetadata { code: 1, name: "NotAuthorized".to_string(), msg: "Not authorized".to_string() },
                ErrorMetadata { code: 2, name: "InvalidExecutor".to_string(), msg: "Invalid executor".to_string() },
            ],
        }
    }

    /// Typed params for `bury`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct BuryParams {
        pub amount: u64,
        /// Optional address override for the `signer` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub signer: Option<String>,
        /// Address of the `sender` account.
        pub sender: String,
        /// Address of the `treasuryOre` account.
        #[serde(rename = "treasuryOre")]
        pub treasury_ore: String,
        /// Address of the `stakeTreasury` account.
        #[serde(rename = "stakeTreasury")]
        pub stake_treasury: String,
        /// Address of the `stakeTreasuryTokens` account.
        #[serde(rename = "stakeTreasuryTokens")]
        pub stake_treasury_tokens: String,
        /// Address of the `stakeVesting` account.
        #[serde(rename = "stakeVesting")]
        pub stake_vesting: String,
        /// Address of the `oreStakeProgram` account.
        #[serde(rename = "oreStakeProgram")]
        pub ore_stake_program: String,
    }

    /// Burns ORE and distributes yield to stakers.
    /// Treasury PDA seeds: ["treasury"].
    pub fn bury(params: BuryParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        bury_handler().build(params)
    }

    /// Raw instruction handler for `bury`.
    pub fn bury_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![24],
            accounts: vec![
                AccountMeta {
                    name: "signer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
                AccountMeta {
                    name: "sender".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "board".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("board".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "mint".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Known("oreoU2P8bN6jkk3jbaiVxYnG1dCXcYxwhwyK9jSybcp".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "treasury".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("treasury".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "treasuryOre".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "stakeTreasury".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "stakeTreasuryTokens".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "stakeVesting".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "tokenProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "oreProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "oreStakeProgram".to_string(),
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
                ErrorMetadata { code: 1, name: "NotAuthorized".to_string(), msg: "Not authorized".to_string() },
                ErrorMetadata { code: 2, name: "InvalidExecutor".to_string(), msg: "Invalid executor".to_string() },
            ],
        }
    }

    /// Typed params for `wrap`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct WrapParams {
        pub amount: u64,
        /// Address of the `treasurySol` account.
        #[serde(rename = "treasurySol")]
        pub treasury_sol: String,
    }

    /// Wraps SOL held by the treasury into WSOL for swapping.
    /// Treasury PDA seeds: ["treasury"].
    pub fn wrap(params: WrapParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        wrap_handler().build(params)
    }

    /// Raw instruction handler for `wrap`.
    pub fn wrap_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![14],
            accounts: vec![
                AccountMeta {
                    name: "signer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Known("HNWhK5f8RMWBqcA7mXJPaxdTPGrha3rrqUrri7HSKb3T".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "config".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("config".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "treasury".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("treasury".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "treasurySol".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "systemProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("11111111111111111111111111111111".to_string()),
                    is_optional: false,
                },
            ],
            args: vec![
                ArgSchema { name: "amount".to_string(), ty: ArgType::U64 },
            ],
            errors: vec![
                ErrorMetadata { code: 0, name: "AmountTooSmall".to_string(), msg: "Amount too small".to_string() },
                ErrorMetadata { code: 1, name: "NotAuthorized".to_string(), msg: "Not authorized".to_string() },
                ErrorMetadata { code: 2, name: "InvalidExecutor".to_string(), msg: "Invalid executor".to_string() },
            ],
        }
    }

    /// Typed params for `setAdmin`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct SetAdminParams {
        pub admin: String,
        /// Optional address override for the `signer` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub signer: Option<String>,
    }

    /// Updates the program admin address.
    pub fn set_admin(params: SetAdminParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        set_admin_handler().build(params)
    }

    /// Raw instruction handler for `setAdmin`.
    pub fn set_admin_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![15],
            accounts: vec![
                AccountMeta {
                    name: "signer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
                AccountMeta {
                    name: "config".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("config".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "systemProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("11111111111111111111111111111111".to_string()),
                    is_optional: false,
                },
            ],
            args: vec![
                ArgSchema { name: "admin".to_string(), ty: ArgType::Pubkey },
            ],
            errors: vec![
                ErrorMetadata { code: 0, name: "AmountTooSmall".to_string(), msg: "Amount too small".to_string() },
                ErrorMetadata { code: 1, name: "NotAuthorized".to_string(), msg: "Not authorized".to_string() },
                ErrorMetadata { code: 2, name: "InvalidExecutor".to_string(), msg: "Invalid executor".to_string() },
            ],
        }
    }

    /// Typed params for `newVar`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct NewVarParams {
        pub id: u64,
        pub commit: Vec<u8>,
        pub samples: u64,
        /// Optional address override for the `signer` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub signer: Option<String>,
        /// Address of the `provider` account.
        pub provider: String,
        /// Address of the `var` account.
        pub var: String,
    }

    /// Creates a new entropy var account through the entropy program.
    pub fn new_var(params: NewVarParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        new_var_handler().build(params)
    }

    /// Raw instruction handler for `newVar`.
    pub fn new_var_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![19],
            accounts: vec![
                AccountMeta {
                    name: "signer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
                AccountMeta {
                    name: "board".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("board".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "config".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::Pda(PdaConfig { program_id: None, seeds: vec![PdaSeed::Literal("config".to_string())] }),
                    is_optional: false,
                },
                AccountMeta {
                    name: "provider".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "var".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "systemProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("11111111111111111111111111111111".to_string()),
                    is_optional: false,
                },
                AccountMeta {
                    name: "entropyProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X".to_string()),
                    is_optional: false,
                },
            ],
            args: vec![
                ArgSchema { name: "id".to_string(), ty: ArgType::U64 },
                ArgSchema { name: "commit".to_string(), ty: ArgType::Array(Box::new(ArgType::U8), 32) },
                ArgSchema { name: "samples".to_string(), ty: ArgType::U64 },
            ],
            errors: vec![
                ErrorMetadata { code: 0, name: "AmountTooSmall".to_string(), msg: "Amount too small".to_string() },
                ErrorMetadata { code: 1, name: "NotAuthorized".to_string(), msg: "Not authorized".to_string() },
                ErrorMetadata { code: 2, name: "InvalidExecutor".to_string(), msg: "Invalid executor".to_string() },
            ],
        }
    }

    /// Typed params for `reloadSol`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct ReloadSolParams {
        /// Optional address override for the `signer` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub signer: Option<String>,
        /// Address of the `automation` account.
        pub automation: String,
        /// Address of the `miner` account.
        pub miner: String,
    }

    /// Deprecated since 3.8.15; this behavior is now included in checkpoint.
    ///
    /// Codegen notes:
    /// - account `automation` degraded to user-provided (PDA 'automation': seed references account 'authority' not present in this instruction)
    /// - account `miner` degraded to user-provided (PDA 'miner': seed references account 'authority' not present in this instruction)
    pub fn reload_sol(params: ReloadSolParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        reload_sol_handler().build(params)
    }

    /// Raw instruction handler for `reloadSol`.
    pub fn reload_sol_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![21],
            accounts: vec![
                AccountMeta {
                    name: "signer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
                // [arete codegen] account `automation` degraded to user-provided (PDA 'automation': seed references account 'authority' not present in this instruction)
                AccountMeta {
                    name: "automation".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                // [arete codegen] account `miner` degraded to user-provided (PDA 'miner': seed references account 'authority' not present in this instruction)
                AccountMeta {
                    name: "miner".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "systemProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("11111111111111111111111111111111".to_string()),
                    is_optional: false,
                },
            ],
            args: vec![],
            errors: vec![
                ErrorMetadata { code: 0, name: "AmountTooSmall".to_string(), msg: "Amount too small".to_string() },
                ErrorMetadata { code: 1, name: "NotAuthorized".to_string(), msg: "Not authorized".to_string() },
                ErrorMetadata { code: 2, name: "InvalidExecutor".to_string(), msg: "Invalid executor".to_string() },
            ],
        }
    }

    /// PDA derivation helpers for this program.
    pub mod pdas {
        use arete_sdk::instruction::{InstructionError, Pubkey, derive_program_address, serialize_seed_value};

        use super::PROGRAM_ID;

        /// Derive the `automation` PDA (returns the address and bump).
        pub fn automation(authority: &str) -> Result<(Pubkey, u8), InstructionError> {
            let seeds: Vec<Vec<u8>> = vec![
                "automation".as_bytes().to_vec(),
                serialize_seed_value(&serde_json::json!(authority), Some("pubkey"))?,
            ];
            derive_program_address(&seeds, PROGRAM_ID)
        }

        /// Derive the `board` PDA (returns the address and bump).
        pub fn board() -> Result<(Pubkey, u8), InstructionError> {
            let seeds: Vec<Vec<u8>> = vec![
                "board".as_bytes().to_vec(),
            ];
            derive_program_address(&seeds, PROGRAM_ID)
        }

        /// Derive the `config` PDA (returns the address and bump).
        pub fn config() -> Result<(Pubkey, u8), InstructionError> {
            let seeds: Vec<Vec<u8>> = vec![
                "config".as_bytes().to_vec(),
            ];
            derive_program_address(&seeds, PROGRAM_ID)
        }

        /// Derive the `miner` PDA (returns the address and bump).
        pub fn miner(authority: &str) -> Result<(Pubkey, u8), InstructionError> {
            let seeds: Vec<Vec<u8>> = vec![
                "miner".as_bytes().to_vec(),
                serialize_seed_value(&serde_json::json!(authority), Some("pubkey"))?,
            ];
            derive_program_address(&seeds, PROGRAM_ID)
        }

        /// Derive the `treasury` PDA (returns the address and bump).
        pub fn treasury() -> Result<(Pubkey, u8), InstructionError> {
            let seeds: Vec<Vec<u8>> = vec![
                "treasury".as_bytes().to_vec(),
            ];
            derive_program_address(&seeds, PROGRAM_ID)
        }
    }

    /// Program accessor exposed on the stack client's `programs` namespace.
    #[derive(Clone)]
    pub struct OreProgram {
        builder: arete_sdk::ProgramBuilder,
    }

    impl OreProgram {
        /// Construct from the connected client's program runtime.
        pub fn from_builder(builder: arete_sdk::ProgramBuilder) -> Self {
            Self { builder }
        }

        pub fn automate(&self, params: AutomateParams) -> Result<BuiltInstruction, InstructionError> {
            automate(params)
        }

        pub fn checkpoint(&self, params: CheckpointParams) -> Result<BuiltInstruction, InstructionError> {
            checkpoint(params)
        }

        pub fn claim_sol(&self, params: ClaimSolParams) -> Result<BuiltInstruction, InstructionError> {
            claim_sol(params)
        }

        pub fn claim_ore(&self, params: ClaimOreParams) -> Result<BuiltInstruction, InstructionError> {
            claim_ore(params)
        }

        pub fn close(&self, params: CloseParams) -> Result<BuiltInstruction, InstructionError> {
            close(params)
        }

        pub fn deploy(&self, params: DeployParams) -> Result<BuiltInstruction, InstructionError> {
            deploy(params)
        }

        pub fn log(&self, params: LogParams) -> Result<BuiltInstruction, InstructionError> {
            log(params)
        }

        pub fn reset(&self, params: ResetParams) -> Result<BuiltInstruction, InstructionError> {
            reset(params)
        }

        pub fn buyback(&self, params: BuybackParams) -> Result<BuiltInstruction, InstructionError> {
            buyback(params)
        }

        pub fn bury(&self, params: BuryParams) -> Result<BuiltInstruction, InstructionError> {
            bury(params)
        }

        pub fn wrap(&self, params: WrapParams) -> Result<BuiltInstruction, InstructionError> {
            wrap(params)
        }

        pub fn set_admin(&self, params: SetAdminParams) -> Result<BuiltInstruction, InstructionError> {
            set_admin(params)
        }

        pub fn new_var(&self, params: NewVarParams) -> Result<BuiltInstruction, InstructionError> {
            new_var(params)
        }

        pub fn reload_sol(&self, params: ReloadSolParams) -> Result<BuiltInstruction, InstructionError> {
            reload_sol(params)
        }

        /// Typed reader for `Automation` accounts (release-addressed HTTP reads).
        pub fn automation_accounts(&self) -> Result<arete_sdk::AccountReader<super::super::types::Automation>, arete_sdk::AreteError> {
            Ok(arete_sdk::AccountReader::new(
                "Automation",
                std::sync::Arc::new(self.builder.account_transport("ore", &read_descriptor())?),
            ))
        }

        /// Typed reader for `Board` accounts (release-addressed HTTP reads).
        pub fn board_accounts(&self) -> Result<arete_sdk::AccountReader<super::super::types::Board>, arete_sdk::AreteError> {
            Ok(arete_sdk::AccountReader::new(
                "Board",
                std::sync::Arc::new(self.builder.account_transport("ore", &read_descriptor())?),
            ))
        }

        /// Typed reader for `Miner` accounts (release-addressed HTTP reads).
        pub fn miner_accounts(&self) -> Result<arete_sdk::AccountReader<super::super::types::Miner>, arete_sdk::AreteError> {
            Ok(arete_sdk::AccountReader::new(
                "Miner",
                std::sync::Arc::new(self.builder.account_transport("ore", &read_descriptor())?),
            ))
        }

        /// Typed reader for `Treasury` accounts (release-addressed HTTP reads).
        pub fn treasury_accounts(&self) -> Result<arete_sdk::AccountReader<super::super::types::Treasury>, arete_sdk::AreteError> {
            Ok(arete_sdk::AccountReader::new(
                "Treasury",
                std::sync::Arc::new(self.builder.account_transport("ore", &read_descriptor())?),
            ))
        }
    }
}

/// Program SDK for `entropy` (program ID `3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X`).
pub mod entropy {
    use arete_sdk::instruction::{AccountMeta, AccountResolution, ArgSchema, ArgType, BuiltInstruction, ErrorMetadata, InstructionError, InstructionHandler};
    use serde::Serialize;

    pub const PROGRAM_ID: &str = "3jSkUuYBoJzQPMEzTvkDFXCZUBksPamrVhrnHR9igu2X";

    /// Content hash of the exact program specification captured at generation time.
    pub const PROGRAM_SPEC_HASH: &str = "arete:h1:program-spec:sha256:b0d48e673ec705cbb6ee41714e660aab9c6398c746b243973fcacd7bc29b7d7b";

    /// Release identity addressing hosted account reads for this program.
    pub const PROGRAM_RELEASE_HASH: &str = "arete:h1:program-release:sha256:9e7d6811735b35f9fd144c1eaa21ac1a48720b706d81bd0d0cd9ad6ec7f32b6c";

    /// Release-addressed read descriptor for this program (HTTP reads over
    /// the client's HTTP base URL).
    pub fn read_descriptor() -> arete_sdk::ProgramReadDescriptor {
        arete_sdk::ProgramReadDescriptor::LocalHttp {
            release: arete_sdk::ProgramReleaseReference {
                program_release_hash: PROGRAM_RELEASE_HASH.to_string(),
                program_spec_hash: PROGRAM_SPEC_HASH.to_string(),
            },
        }
    }

    /// Typed params for `open`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct OpenParams {
        pub id: u64,
        pub commit: Vec<u8>,
        #[serde(rename = "isAuto")]
        pub is_auto: u64,
        pub samples: u64,
        #[serde(rename = "endAt")]
        pub end_at: u64,
        /// Optional address override for the `authority` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub authority: Option<String>,
        /// Optional address override for the `payer` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub payer: Option<String>,
        /// Address of the `provider` account.
        pub provider: String,
        /// Address of the `var` account.
        pub var: String,
    }

    /// Creates a new entropy var account.
    /// Var PDA seeds: ["var", authority, id].
    pub fn open(params: OpenParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        open_handler().build(params)
    }

    /// Raw instruction handler for `open`.
    pub fn open_handler() -> InstructionHandler {
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
                    name: "payer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
                AccountMeta {
                    name: "provider".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "var".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "systemProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("11111111111111111111111111111111".to_string()),
                    is_optional: false,
                },
            ],
            args: vec![
                ArgSchema { name: "id".to_string(), ty: ArgType::U64 },
                ArgSchema { name: "commit".to_string(), ty: ArgType::Array(Box::new(ArgType::U8), 32) },
                ArgSchema { name: "isAuto".to_string(), ty: ArgType::U64 },
                ArgSchema { name: "samples".to_string(), ty: ArgType::U64 },
                ArgSchema { name: "endAt".to_string(), ty: ArgType::U64 },
            ],
            errors: vec![
                ErrorMetadata { code: 0, name: "IncompleteDigest".to_string(), msg: "Incomplete digest".to_string() },
                ErrorMetadata { code: 1, name: "InvalidSeed".to_string(), msg: "Invalid seed".to_string() },
            ],
        }
    }

    /// Typed params for `close`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct CloseParams {
        /// Optional address override for the `signer` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub signer: Option<String>,
        /// Address of the `var` account.
        pub var: String,
    }

    /// Closes an entropy var account and returns rent to the authority.
    pub fn close(params: CloseParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        close_handler().build(params)
    }

    /// Raw instruction handler for `close`.
    pub fn close_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![1],
            accounts: vec![
                AccountMeta {
                    name: "signer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
                AccountMeta {
                    name: "var".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "systemProgram".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("11111111111111111111111111111111".to_string()),
                    is_optional: false,
                },
            ],
            args: vec![],
            errors: vec![
                ErrorMetadata { code: 0, name: "IncompleteDigest".to_string(), msg: "Incomplete digest".to_string() },
                ErrorMetadata { code: 1, name: "InvalidSeed".to_string(), msg: "Invalid seed".to_string() },
            ],
        }
    }

    /// Typed params for `next`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct NextParams {
        #[serde(rename = "endAt")]
        pub end_at: u64,
        /// Optional address override for the `signer` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub signer: Option<String>,
        /// Address of the `var` account.
        pub var: String,
    }

    /// Updates the var for the next random value sample.
    /// Resets the commit to the previous seed and clears slot_hash, seed, and value.
    pub fn next(params: NextParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        next_handler().build(params)
    }

    /// Raw instruction handler for `next`.
    pub fn next_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![2],
            accounts: vec![
                AccountMeta {
                    name: "signer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
                AccountMeta {
                    name: "var".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
            ],
            args: vec![
                ArgSchema { name: "endAt".to_string(), ty: ArgType::U64 },
            ],
            errors: vec![
                ErrorMetadata { code: 0, name: "IncompleteDigest".to_string(), msg: "Incomplete digest".to_string() },
                ErrorMetadata { code: 1, name: "InvalidSeed".to_string(), msg: "Invalid seed".to_string() },
            ],
        }
    }

    /// Typed params for `reveal`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct RevealParams {
        pub seed: Vec<u8>,
        /// Optional address override for the `signer` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub signer: Option<String>,
        /// Address of the `var` account.
        pub var: String,
    }

    /// Reveals the seed and finalizes the random value.
    /// The seed must hash to the commit stored in the var account.
    pub fn reveal(params: RevealParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        reveal_handler().build(params)
    }

    /// Raw instruction handler for `reveal`.
    pub fn reveal_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![4],
            accounts: vec![
                AccountMeta {
                    name: "signer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
                AccountMeta {
                    name: "var".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
            ],
            args: vec![
                ArgSchema { name: "seed".to_string(), ty: ArgType::Array(Box::new(ArgType::U8), 32) },
            ],
            errors: vec![
                ErrorMetadata { code: 0, name: "IncompleteDigest".to_string(), msg: "Incomplete digest".to_string() },
                ErrorMetadata { code: 1, name: "InvalidSeed".to_string(), msg: "Invalid seed".to_string() },
            ],
        }
    }

    /// Typed params for `sample`: instruction args plus overridable accounts.
    #[derive(Debug, Clone, Serialize, Default)]
    pub struct SampleParams {
        /// Optional address override for the `signer` signer (defaults to the payer).
        #[serde(skip_serializing_if = "Option::is_none")]
        pub signer: Option<String>,
        /// Address of the `var` account.
        pub var: String,
    }

    /// Samples the slot hash at the end_at slot.
    /// Must be called after the end_at slot has passed.
    pub fn sample(params: SampleParams) -> Result<BuiltInstruction, InstructionError> {
        let params = serde_json::to_value(params).map_err(|error| InstructionError::InvalidValue {
            context: "params".to_string(),
            message: error.to_string(),
        })?;
        sample_handler().build(params)
    }

    /// Raw instruction handler for `sample`.
    pub fn sample_handler() -> InstructionHandler {
        InstructionHandler {
            program_id: PROGRAM_ID.to_string(),
            discriminator: vec![5],
            accounts: vec![
                AccountMeta {
                    name: "signer".to_string(),
                    is_signer: true,
                    is_writable: true,
                    resolution: AccountResolution::Signer,
                    is_optional: false,
                },
                AccountMeta {
                    name: "var".to_string(),
                    is_signer: false,
                    is_writable: true,
                    resolution: AccountResolution::UserProvided,
                    is_optional: false,
                },
                AccountMeta {
                    name: "slotHashesSysvar".to_string(),
                    is_signer: false,
                    is_writable: false,
                    resolution: AccountResolution::Known("SysvarS1otHashes111111111111111111111111111".to_string()),
                    is_optional: false,
                },
            ],
            args: vec![],
            errors: vec![
                ErrorMetadata { code: 0, name: "IncompleteDigest".to_string(), msg: "Incomplete digest".to_string() },
                ErrorMetadata { code: 1, name: "InvalidSeed".to_string(), msg: "Invalid seed".to_string() },
            ],
        }
    }

    /// Program accessor exposed on the stack client's `programs` namespace.
    #[derive(Clone)]
    pub struct EntropyProgram {
        #[allow(dead_code)]
        builder: arete_sdk::ProgramBuilder,
    }

    impl EntropyProgram {
        /// Construct from the connected client's program runtime.
        pub fn from_builder(builder: arete_sdk::ProgramBuilder) -> Self {
            Self { builder }
        }

        pub fn open(&self, params: OpenParams) -> Result<BuiltInstruction, InstructionError> {
            open(params)
        }

        pub fn close(&self, params: CloseParams) -> Result<BuiltInstruction, InstructionError> {
            close(params)
        }

        pub fn next(&self, params: NextParams) -> Result<BuiltInstruction, InstructionError> {
            next(params)
        }

        pub fn reveal(&self, params: RevealParams) -> Result<BuiltInstruction, InstructionError> {
            reveal(params)
        }

        pub fn sample(&self, params: SampleParams) -> Result<BuiltInstruction, InstructionError> {
            sample(params)
        }
    }
}
