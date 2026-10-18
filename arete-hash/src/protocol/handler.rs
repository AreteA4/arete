//! The `arete.protocol-handler/v1` wire ABI: the input a protocol handler
//! receives for a step and the operation it returns.
//!
//! A handler returns portable instructions only. It never returns signers,
//! fees, memos or compiled messages; trusted code adds those after
//! validating the operation against the step's declarations.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::error::{ProtocolContractError, ProtocolResult};
use super::intent::{AccountReadResultV1, FeePlacement};
use super::model::PROTOCOL_HANDLER_ABI_V1;
use super::release::decode_base64;
use super::shape::*;

pub const MAX_OPERATION_TRANSACTIONS: usize = 8;
pub const MAX_TRANSACTION_INSTRUCTIONS: usize = 64;
pub const MAX_INSTRUCTION_ACCOUNTS: usize = 256;
pub const MAX_INSTRUCTION_DATA_BYTES: usize = 10 * 1024;
pub const MAX_HANDLER_RESULT_BYTES: usize = 2 * 1024 * 1024;

/// What the API hands the host for one invocation. Every value is fixed by
/// the API before dispatch; handlers have no clock, RPC or randomness.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareInputV1 {
    pub abi: String,
    pub step: String,
    pub args: Value,
    pub resources: BTreeMap<String, Value>,
    pub params: Value,
    pub instance: Option<InstanceInputV1>,
    pub participant: Option<ParticipantInputV1>,
    pub reads: BTreeMap<String, ReadInputV1>,
    pub wallet: String,
    pub execution_id: String,
    pub ephemeral: BTreeMap<String, String>,
    pub programs: BTreeMap<String, String>,
    pub fee: Option<FeeContextV1>,
    /// Unix seconds, fixed for the whole invocation.
    pub now: i64,
    /// The platform keys of the deployment the invocation runs under.
    pub deployment: DeploymentInputV1,
}

/// The deployment an invocation runs under and the platform keys it names.
/// The same values are the `deployment.*` expression root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentInputV1 {
    /// The deployment id (a UUID).
    pub id: String,
    /// The deployment's attestation pubkey.
    pub attestation: String,
    /// The Squads proposer pubkey for the calling integrator account, or
    /// `None` when the deployment has no `squadsProposer` step.
    pub squads_proposer: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InstanceInputV1 {
    pub id: String,
    pub state: String,
    pub accounts: BTreeMap<String, String>,
    pub roles: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParticipantInputV1 {
    pub id: String,
    pub wallet: String,
    pub state: String,
}

/// A declared read: the raw account plus, for `account` reads, the fields
/// decoded with the program's ProgramSpec.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReadInputV1 {
    pub account: AccountReadResultV1,
    pub decoded: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FeeContextV1 {
    pub gross: String,
    pub fee: String,
    pub net: String,
    /// `"native"` or a mint address.
    pub asset: String,
    pub placement: FeePlacement,
}

/// What a handler returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedOperationV1 {
    pub abi: String,
    pub transactions: Vec<PreparedTransactionV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparedTransactionV1 {
    pub name: String,
    pub instructions: Vec<PortableInstructionV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PortableInstructionV1 {
    pub program_id: String,
    pub keys: Vec<PortableAccountMetaV1>,
    pub data_base64: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PortableAccountMetaV1 {
    pub pubkey: String,
    pub is_signer: bool,
    pub is_writable: bool,
}

impl PortableInstructionV1 {
    pub fn data(&self) -> Vec<u8> {
        decode_base64(&self.data_base64).unwrap_or_default()
    }
}

/// Validate a handler's result shape and size bounds. Program, account and
/// signer policy is checked afterwards against the step by the trusted
/// transaction compiler.
pub fn parse_prepared_operation_v1(bytes: &[u8]) -> ProtocolResult<PreparedOperationV1> {
    if bytes.len() > MAX_HANDLER_RESULT_BYTES {
        return Err(ProtocolContractError::new(
            "string-too-long",
            "",
            format!("handler results are at most {MAX_HANDLER_RESULT_BYTES} bytes"),
        ));
    }
    let value = super::parse_contract_json(bytes)?;
    validate_prepared_operation_value(&value)
}

pub fn validate_prepared_operation_value(value: &Value) -> ProtocolResult<PreparedOperationV1> {
    reject_floats(value, "")?;
    let root = Obj::new(value, "", &["abi", "transactions"])?;
    schema_tag(root.req("abi")?, "/abi", PROTOCOL_HANDLER_ABI_V1)?;
    let transactions = as_array(
        root.req("transactions")?,
        "/transactions",
        MAX_OPERATION_TRANSACTIONS,
    )?;
    if transactions.is_empty() {
        return Err(err(
            "invalid-value",
            "/transactions",
            "an operation has at least one transaction",
        ));
    }
    for (i, transaction) in transactions.iter().enumerate() {
        let path = index("/transactions", i);
        let transaction = Obj::new(transaction, &path, &["name", "instructions"])?;
        identifier_value(transaction.req("name")?, &transaction.path_of("name"))?;
        let instructions_path = transaction.path_of("instructions");
        let instructions = as_array(
            transaction.req("instructions")?,
            &instructions_path,
            MAX_TRANSACTION_INSTRUCTIONS,
        )?;
        if instructions.is_empty() {
            return Err(err(
                "invalid-value",
                &instructions_path,
                "a transaction has at least one instruction",
            ));
        }
        for (j, instruction) in instructions.iter().enumerate() {
            let path = index(&instructions_path, j);
            let instruction = Obj::new(instruction, &path, &["programId", "keys", "dataBase64"])?;
            pubkey(
                instruction.req("programId")?,
                &instruction.path_of("programId"),
            )?;
            let keys_path = instruction.path_of("keys");
            for (k, key) in as_array(
                instruction.req("keys")?,
                &keys_path,
                MAX_INSTRUCTION_ACCOUNTS,
            )?
            .iter()
            .enumerate()
            {
                let key = Obj::new(
                    key,
                    &index(&keys_path, k),
                    &["pubkey", "isSigner", "isWritable"],
                )?;
                pubkey(key.req("pubkey")?, &key.path_of("pubkey"))?;
                as_bool(key.req("isSigner")?, &key.path_of("isSigner"))?;
                as_bool(key.req("isWritable")?, &key.path_of("isWritable"))?;
            }
            let data_path = instruction.path_of("dataBase64");
            let data = as_str(instruction.req("dataBase64")?, &data_path)?;
            match decode_base64(data) {
                Some(bytes) if bytes.len() <= MAX_INSTRUCTION_DATA_BYTES => {}
                Some(_) => {
                    return Err(err(
                        "string-too-long",
                        &data_path,
                        "instruction data is too large",
                    ))
                }
                None => {
                    return Err(err(
                        "invalid-value",
                        &data_path,
                        "expected canonical base64",
                    ))
                }
            }
        }
    }
    serde_json::from_value(value.clone()).map_err(|error| {
        ProtocolContractError::new("invalid-value", "", format!("model mismatch: {error}"))
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn input(deployment: Value) -> Value {
        json!({
            "abi": PROTOCOL_HANDLER_ABI_V1, "step": "create", "args": {}, "resources": {},
            "params": {}, "instance": null, "participant": null, "reads": {},
            "wallet": "11111111111111111111111111111111",
            "executionId": "00000000-0000-4000-8000-000000000001",
            "ephemeral": {}, "programs": {}, "fee": null, "now": 1_789_000_000,
            "deployment": deployment,
        })
    }

    #[test]
    fn prepare_input_carries_the_deployment_keys() {
        let parsed: PrepareInputV1 = serde_json::from_value(input(json!({
            "id": "00000000-0000-4000-8000-0000000000d1",
            "attestation": "Vote111111111111111111111111111111111111111",
            "squadsProposer": null,
        })))
        .expect("valid input");
        assert_eq!(parsed.deployment.squads_proposer, None);
        let wire = serde_json::to_value(&parsed).expect("serializes");
        assert_eq!(wire["deployment"]["squadsProposer"], Value::Null);
        assert_eq!(
            wire["deployment"]["attestation"],
            "Vote111111111111111111111111111111111111111"
        );

        let mut missing = input(Value::Null);
        missing
            .as_object_mut()
            .expect("object")
            .remove("deployment");
        assert!(serde_json::from_value::<PrepareInputV1>(missing).is_err());
        let extra = input(json!({
            "id": "x", "attestation": "y", "squadsProposer": null, "other": 1,
        }));
        assert!(serde_json::from_value::<PrepareInputV1>(extra).is_err());
    }
}
