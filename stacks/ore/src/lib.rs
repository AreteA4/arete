mod stack;

pub use stack::ore_stream::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_spec_registers_ore_and_entropy_releases() {
        let spec = spec();
        assert_eq!(spec.program_runtime_definitions.len(), 2);
        assert_eq!(__ARETE_OSS_PROGRAM_RELEASE_HASHES_V1.len(), 2);
        assert_eq!(__ARETE_PROGRAM_SPEC_HASHES_V1.len(), 2);

        for ((definition, expected_release), expected_spec) in spec
            .program_runtime_definitions
            .iter()
            .zip(__ARETE_OSS_PROGRAM_RELEASE_HASHES_V1)
            .zip(__ARETE_PROGRAM_SPEC_HASHES_V1)
        {
            assert_eq!(
                definition.program_release_hash.to_string(),
                *expected_release
            );
            assert_eq!(definition.program_spec_hash.to_string(), *expected_spec);
            assert!(spec.program_ids.contains(&definition.program_id));
        }
        assert_ne!(
            spec.program_runtime_definitions[0].program_id,
            spec.program_runtime_definitions[1].program_id
        );
    }
}
