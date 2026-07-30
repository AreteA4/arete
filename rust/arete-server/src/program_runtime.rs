use anyhow::{bail, Result};
use std::collections::HashMap;
use std::sync::Arc;

pub type ProgramSpecHash = arete_hash::HashId<arete_hash::ProgramSpec>;
pub type IdlContentHash = arete_hash::HashId<arete_hash::IdlContent>;
pub type NormalizedIdlHash = arete_hash::HashId<arete_hash::IdlNormalized>;
pub type ProgramReleaseHash = arete_hash::HashId<arete_hash::ProgramRelease>;

/// A generated account decoder bound to one program release.
///
/// Macro-generated readers accept trailing account bytes, matching live decoder
/// behavior, but never expose those bytes in their JSON result.
pub type ProgramAccountReaderFn =
    Arc<dyn Fn(&str, &[u8]) -> Result<serde_json::Value> + Send + Sync>;

#[derive(Clone)]
pub struct ProgramRuntimeDefinition {
    pub program_id: String,
    pub program_spec_hash: ProgramSpecHash,
    pub idl_content_hash: IdlContentHash,
    pub normalized_idl_hash: NormalizedIdlHash,
    pub program_release_hash: ProgramReleaseHash,
    pub account_reader: ProgramAccountReaderFn,
}

impl ProgramRuntimeDefinition {
    fn validate(&self) -> Result<()> {
        let expected = arete_hash::OssGeneratedProgramReleaseV1::new(
            self.program_id.clone(),
            self.program_spec_hash,
            self.idl_content_hash,
            self.normalized_idl_hash,
        )
        .hash()?;
        if expected != self.program_release_hash {
            bail!(
                "program runtime definition release hash does not match its public identity fields"
            );
        }
        Ok(())
    }

    fn is_exact_duplicate_of(&self, other: &Self) -> bool {
        self.program_id == other.program_id
            && self.program_spec_hash == other.program_spec_hash
            && self.idl_content_hash == other.idl_content_hash
            && self.normalized_idl_hash == other.normalized_idl_hash
            && Arc::ptr_eq(&self.account_reader, &other.account_reader)
    }
}

#[derive(Clone, Default)]
pub struct ProgramRuntimeCatalog {
    definitions: HashMap<ProgramReleaseHash, ProgramRuntimeDefinition>,
}

impl ProgramRuntimeCatalog {
    pub fn try_new(definitions: Vec<ProgramRuntimeDefinition>) -> Result<Self> {
        let mut catalog = Self::default();
        for definition in definitions {
            definition.validate()?;
            if let Some(existing) = catalog.definitions.get(&definition.program_release_hash) {
                if !existing.is_exact_duplicate_of(&definition) {
                    bail!("conflicting program runtime definitions use the same release hash");
                }
                continue;
            }
            catalog
                .definitions
                .insert(definition.program_release_hash, definition);
        }
        Ok(catalog)
    }

    pub fn get(&self, release_hash: &ProgramReleaseHash) -> Option<&ProgramRuntimeDefinition> {
        self.definitions.get(release_hash)
    }

    pub fn len(&self) -> usize {
        self.definitions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(program_id: &str, reader: ProgramAccountReaderFn) -> ProgramRuntimeDefinition {
        let program_spec_hash = ProgramSpecHash::from_digest([1; 32]);
        let idl_content_hash = IdlContentHash::from_digest([2; 32]);
        let normalized_idl_hash = NormalizedIdlHash::from_digest([3; 32]);
        let program_release_hash = arete_hash::OssGeneratedProgramReleaseV1::new(
            program_id,
            program_spec_hash,
            idl_content_hash,
            normalized_idl_hash,
        )
        .hash()
        .unwrap();
        ProgramRuntimeDefinition {
            program_id: program_id.to_string(),
            program_spec_hash,
            idl_content_hash,
            normalized_idl_hash,
            program_release_hash,
            account_reader: reader,
        }
    }

    #[test]
    fn catalog_accepts_exact_duplicates_and_rejects_conflicts() {
        let reader: ProgramAccountReaderFn = Arc::new(|_, _| Ok(serde_json::Value::Null));
        let first = definition("Program111", reader.clone());
        let duplicate = first.clone();
        assert_eq!(
            ProgramRuntimeCatalog::try_new(vec![first.clone(), duplicate])
                .unwrap()
                .len(),
            1
        );

        let mut conflict = first.clone();
        conflict.account_reader = Arc::new(|_, _| Ok(serde_json::Value::Bool(true)));
        assert!(ProgramRuntimeCatalog::try_new(vec![first, conflict]).is_err());
    }

    #[test]
    fn catalog_rejects_a_release_hash_mismatch() {
        let reader: ProgramAccountReaderFn = Arc::new(|_, _| Ok(serde_json::Value::Null));
        let mut definition = definition("Program111", reader);
        definition.program_release_hash = ProgramReleaseHash::from_digest([9; 32]);
        assert!(ProgramRuntimeCatalog::try_new(vec![definition]).is_err());
    }
}
