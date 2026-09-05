use std::collections::HashSet;
use std::str::FromStr;

use arete_hash::*;
use serde_json::Value;
use sha2::{Digest, Sha256};

fn corpus() -> Value {
    serde_json::from_str(include_str!("../../test-vectors/hash-v1.json"))
        .expect("hash-v1 vector corpus must be valid JSON")
}

fn input_bytes(input: &Value) -> Vec<u8> {
    match input["encoding"].as_str().expect("input encoding") {
        "utf8" => input["data"]
            .as_str()
            .expect("UTF-8 data")
            .as_bytes()
            .to_vec(),
        "hex" => hex::decode(input["data"].as_str().expect("hex data")).expect("valid hex"),
        other => panic!("unsupported byte input encoding '{other}'"),
    }
}

fn vector_input(input: &Value) -> VectorInput {
    match input["encoding"].as_str().expect("input encoding") {
        "utf8" | "hex" => VectorInput::RawBytes(input_bytes(input)),
        "tuple" => VectorInput::TupleFields(
            input["fields"]
                .as_array()
                .expect("tuple fields")
                .iter()
                .map(|field| {
                    let value = if let Some(value) = field.get("valueUtf8") {
                        value
                            .as_str()
                            .expect("UTF-8 tuple value")
                            .as_bytes()
                            .to_vec()
                    } else {
                        hex::decode(field["valueHex"].as_str().expect("hex tuple value"))
                            .expect("valid tuple hex")
                    };
                    (
                        field["label"].as_str().expect("tuple label").to_string(),
                        value,
                    )
                })
                .collect(),
        ),
        "tree" => VectorInput::TreeEntries(
            input["entries"]
                .as_array()
                .expect("tree entries")
                .iter()
                .map(|entry| VectorTreeEntry {
                    path: entry["path"].as_str().expect("tree path").to_string(),
                    bytes: hex::decode(entry["bytesHex"].as_str().expect("tree bytes"))
                        .expect("valid tree bytes"),
                    symlink: entry["type"].as_str() == Some("symlink"),
                })
                .collect(),
        ),
        other => panic!("unsupported vector input encoding '{other}'"),
    }
}

fn assert_expected(expected: &Value, outcome: &VectorOutcome) {
    assert_eq!(
        expected["canonicalPayloadHex"].as_str(),
        Some(hex::encode(&outcome.canonical_payload).as_str())
    );
    assert_eq!(
        expected["preimageHex"].as_str(),
        Some(hex::encode(&outcome.preimage).as_str())
    );
    assert_eq!(
        expected["digestHex"].as_str(),
        Some(hex::encode(outcome.digest).as_str())
    );
    assert_eq!(
        expected["hashId"].as_str(),
        Some(outcome.hash_id.to_string().as_str())
    );
}

fn assert_expected_identity(
    expected: &Value,
    kind: HashKindName,
    profile: CanonicalizationProfile,
    payload: &[u8],
    hash_id: AnyHashId,
) {
    let preimage = framed_preimage(kind, profile, payload);
    let digest: [u8; 32] = Sha256::digest(&preimage).into();
    assert_eq!(hash_id.kind(), kind);
    assert_eq!(hash_id.digest(), &digest);
    assert_eq!(expected["canonicalPayloadHex"], hex::encode(payload));
    assert_eq!(expected["preimageHex"], hex::encode(preimage));
    assert_eq!(expected["digestHex"], hex::encode(digest));
    assert_eq!(expected["hashId"], hash_id.to_string());
}

#[test]
fn checked_in_hash_vectors_conform_byte_for_byte() {
    let corpus = corpus();
    assert_eq!(corpus["schema"], "arete.hash-vectors/v1");
    assert_eq!(corpus["protocol"]["label"], HASH_PROTOCOL_LABEL);
    assert_eq!(corpus["protocol"]["version"], HASH_PROTOCOL_VERSION);
    assert_eq!(corpus["protocol"]["algorithm"], HASH_ALGORITHM);

    for vector in corpus["hashVectors"].as_array().expect("hash vectors") {
        let kind =
            HashKindName::from_str(vector["kind"].as_str().expect("kind")).expect("known kind");
        let profile =
            CanonicalizationProfile::from_str(vector["profile"].as_str().expect("profile"))
                .expect("known profile");
        let outcome = execute_vector(kind, profile, &vector_input(&vector["input"]))
            .unwrap_or_else(|error| panic!("vector '{}' failed: {error}", vector["id"]));
        assert_expected(&vector["expected"], &outcome);
    }
}

#[test]
fn checked_in_failure_vectors_fail_with_stable_codes() {
    let corpus = corpus();
    for vector in corpus["failureVectors"]
        .as_array()
        .expect("failure vectors")
    {
        let input = &vector["input"];
        let result: Result<(), HashError> = match vector["operation"].as_str().expect("operation") {
            "arete-jcs-v1" => canonicalize_json_bytes(&input_bytes(input)).map(|_| ()),
            "framed-tuple-v1" => match vector_input(input) {
                VectorInput::TupleFields(fields) => {
                    let borrowed: Vec<_> = fields
                        .iter()
                        .map(|(label, value)| TupleField::new(label, value))
                        .collect();
                    framed_tuple_payload(&borrowed).map(|_| ())
                }
                _ => unreachable!(),
            },
            "artifact-tree-v1" => match vector_input(input) {
                VectorInput::TreeEntries(entries) => {
                    let borrowed: Vec<_> = entries
                        .iter()
                        .map(|entry| ArtifactTreeEntry {
                            path: &entry.path,
                            bytes: &entry.bytes,
                            kind: if entry.symlink {
                                ArtifactEntryKind::Symlink
                            } else {
                                ArtifactEntryKind::File
                            },
                        })
                        .collect();
                    artifact_tree_payload(&borrowed).map(|_| ())
                }
                _ => unreachable!(),
            },
            "execute-vector" => execute_vector(
                HashKindName::from_str(input["kind"].as_str().unwrap()).unwrap(),
                CanonicalizationProfile::from_str(input["profile"].as_str().unwrap()).unwrap(),
                &vector_input(&input["input"]),
            )
            .map(|_| ()),
            "parse-profile" => {
                CanonicalizationProfile::from_str(input["profile"].as_str().unwrap()).map(|_| ())
            }
            "remove-artifact-hash" => project_without_artifact_hash(&input["value"]).map(|_| ()),
            "program-spec-v1" => {
                let mut projection: ProgramSpecV1 =
                    serde_json::from_value(input["projection"].clone()).unwrap();
                if let Some(version) = input
                    .get("normalizationVersionOverride")
                    .and_then(Value::as_u64)
                {
                    projection.idl_snapshot.normalization_version = version as u32;
                }
                projection.hash().map(|_| ())
            }
            "oss-program-release-v1" => {
                serde_json::from_value::<OssGeneratedProgramReleaseV1>(input["projection"].clone())
                    .unwrap()
                    .hash()
                    .map(|_| ())
            }
            "decoder-fixture-set-v2" => {
                parse_decoder_fixture_set_v2(&serde_json::to_vec(&input["projection"]).unwrap())
                    .map(|_| ())
            }
            "hosted-program-release-v2" => parse_hosted_managed_program_release_v2(
                &serde_json::to_vec(&input["projection"]).unwrap(),
            )
            .map(|_| ()),
            "hosted-program-release-v3" => parse_hosted_private_program_release_v3(
                &serde_json::to_vec(&input["projection"]).unwrap(),
            )
            .map(|_| ()),
            other => panic!("unknown failure operation '{other}'"),
        };
        let error = result.unwrap_err();
        assert_eq!(
            error.code(),
            vector["expectedError"].as_str().unwrap(),
            "failure vector '{}'",
            vector["id"]
        );
    }
}

#[test]
fn checked_in_hash_id_vectors_fail_closed() {
    let corpus = corpus();
    for vector in corpus["hashIdVectors"].as_array().expect("HashId vectors") {
        let parsed = AnyHashId::from_str(vector["input"].as_str().expect("HashId input"));
        if vector["valid"] == true {
            let parsed = parsed.expect("valid HashId");
            assert_eq!(parsed.kind().as_str(), vector["expectedKind"]);
            assert_eq!(hex::encode(parsed.digest()), vector["expectedDigestHex"]);
        } else {
            assert_eq!(
                parsed.unwrap_err().code(),
                vector["expectedError"].as_str().unwrap()
            );
        }
    }
}

#[test]
fn checked_in_self_hash_vectors_remove_only_the_declared_field() {
    let corpus = corpus();
    for vector in corpus["selfHashVectors"]
        .as_array()
        .expect("self-hash vectors")
    {
        let projection =
            project_without_artifact_hash(&vector["input"]).expect("object projection");
        assert_eq!(projection, vector["expectedProjection"]);
        let payload = canonicalize_jcs(&projection).expect("projection canonicalizes");
        let hash = hash_jcs::<AstPortable, _>(&projection).expect("projection hashes");
        assert_expected_identity(
            &vector["expected"],
            HashKindName::AstPortable,
            CanonicalizationProfile::AreteJcsV1,
            &payload,
            hash.into_any(),
        );
    }
}

#[test]
fn checked_in_idl_vectors_derive_all_projections_and_release_identity() {
    let corpus = corpus();
    for vector in corpus["idlVectors"].as_array().expect("IDL vectors") {
        let input = &vector["input"];
        let source = input_bytes(input);
        let explicit = input.get("explicitProgramId").and_then(Value::as_str);
        let document = CanonicalIdlDocument::parse(&source, explicit)
            .unwrap_or_else(|error| panic!("IDL vector '{}' failed: {error}", vector["id"]));
        let identity = OssProgramIdentityV1::from_document(&document).expect("OSS identity");
        let expected = &vector["expected"];

        assert_eq!(document.program_id(), expected["programId"]);
        assert_eq!(
            document.content_projection(),
            &expected["contentProjection"]
        );
        assert_eq!(
            document.portable_projection(),
            &expected["portableProjection"]
        );
        assert_eq!(
            serde_json::to_value(document.normalized_snapshot()).unwrap(),
            expected["normalizedSnapshot"]
        );
        assert_eq!(
            serde_json::to_value(&identity.program_spec).unwrap(),
            expected["programSpec"]
        );
        assert_eq!(
            serde_json::to_value(&identity.release).unwrap(),
            expected["ossRelease"]
        );

        assert_expected_identity(
            &expected["source"],
            HashKindName::IdlSource,
            CanonicalizationProfile::RawBytesV1,
            document.source_bytes(),
            document.hashes().source.into_any(),
        );
        assert_expected_identity(
            &expected["content"],
            HashKindName::IdlContent,
            CanonicalizationProfile::AreteJcsV1,
            &document.content_payload().unwrap(),
            document.hashes().content.into_any(),
        );
        assert_expected_identity(
            &expected["portable"],
            HashKindName::IdlPortable,
            CanonicalizationProfile::AreteJcsV1,
            &document.portable_payload().unwrap(),
            document.hashes().portable.into_any(),
        );
        assert_expected_identity(
            &expected["normalized"],
            HashKindName::IdlNormalized,
            CanonicalizationProfile::AreteJcsV1,
            &document.normalized_payload().unwrap(),
            document.hashes().normalized.into_any(),
        );
        assert_expected_identity(
            &expected["programSpecIdentity"],
            HashKindName::ProgramSpec,
            CanonicalizationProfile::AreteJcsV1,
            &canonicalize_jcs(&identity.program_spec).unwrap(),
            identity.program_spec_hash.into_any(),
        );
        assert_expected_identity(
            &expected["ossReleaseIdentity"],
            HashKindName::ProgramRelease,
            CanonicalizationProfile::AreteJcsV1,
            &canonicalize_jcs(&identity.release).unwrap(),
            identity.release_hash.into_any(),
        );
    }
}

#[test]
fn checked_in_idl_failure_vectors_reject_conflicts_and_invalid_sources() {
    let corpus = corpus();
    for vector in corpus["idlFailureVectors"]
        .as_array()
        .expect("IDL failures")
    {
        let input = &vector["input"];
        let source = input_bytes(input);
        let explicit = input.get("explicitProgramId").and_then(Value::as_str);
        let error = CanonicalIdlDocument::parse(&source, explicit).unwrap_err();
        assert_eq!(error.code(), vector["expectedError"].as_str().unwrap());
    }
}

#[test]
fn checked_in_release_vectors_validate_typed_projections() {
    let corpus = corpus();
    for vector in corpus["releaseVectors"]
        .as_array()
        .expect("release vectors")
    {
        let projection = &vector["projection"];
        let hash = match (
            projection["schema"].as_str().unwrap(),
            projection["releaseProfile"].as_str().unwrap(),
        ) {
            (PROGRAM_RELEASE_SCHEMA_V1, OSS_GENERATED_RELEASE_PROFILE) => {
                serde_json::from_value::<OssGeneratedProgramReleaseV1>(projection.clone())
                    .expect("OSS projection")
                    .hash()
                    .expect("valid OSS projection")
            }
            (PROGRAM_RELEASE_SCHEMA_V2, HOSTED_MANAGED_RELEASE_PROFILE) => {
                parse_hosted_managed_program_release_v2(&serde_json::to_vec(projection).unwrap())
                    .expect("hosted V2 projection")
                    .hash()
                    .expect("valid hosted V2 projection")
            }
            (PROGRAM_RELEASE_SCHEMA_V3, HOSTED_PRIVATE_RELEASE_PROFILE) => {
                parse_hosted_private_program_release_v3(&serde_json::to_vec(projection).unwrap())
                    .expect("hosted-private V3 projection")
                    .hash()
                    .expect("valid hosted-private V3 projection")
            }
            (PROGRAM_RELEASE_SCHEMA_V1, HOSTED_MANAGED_RELEASE_PROFILE) => {
                hash_jcs::<ProgramRelease, _>(projection).expect("historical hosted V1 hashes")
            }
            (schema, profile) => panic!("unknown release projection '{schema}'/'{profile}'"),
        };
        let payload = canonicalize_jcs(projection).unwrap();
        assert_expected_identity(
            &vector["expected"],
            HashKindName::ProgramRelease,
            CanonicalizationProfile::AreteJcsV1,
            &payload,
            hash.into_any(),
        );
    }
}

#[test]
fn hosted_private_release_v3_excludes_access_metadata_and_hashes_every_identity_field() {
    let corpus = corpus();
    let vectors = corpus["releaseVectors"].as_array().unwrap();
    let baseline = vectors
        .iter()
        .find(|vector| vector["id"] == "release-hosted-private-v3-observed")
        .expect("shared hosted-private vector");
    let projection = baseline["projection"].as_object().unwrap();
    for forbidden in [
        "ownerUserId",
        "visibility",
        "alias",
        "admissionId",
        "executableIdentity",
        "objectKey",
    ] {
        assert!(!projection.contains_key(forbidden));
    }
    let baseline_hash = baseline["expected"]["hashId"].as_str().unwrap();
    for id in [
        "release-hosted-private-v3-program-id-change",
        "release-hosted-private-v3-abi-change",
        "release-hosted-private-v3-engine-change",
        "release-hosted-private-v3-binding-change",
    ] {
        let changed = vectors.iter().find(|vector| vector["id"] == id).unwrap();
        assert_ne!(changed["expected"]["hashId"], baseline_hash);
    }
}

#[test]
fn hosted_release_v2_without_upgrade_authority_matches_shared_bytes_and_hash() {
    let corpus = corpus();
    let vector = corpus["releaseVectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|vector| vector["id"] == "release-hosted-managed-v2-upgradeable-no-authority")
        .expect("shared no-authority release vector");
    assert_eq!(
        vector["projection"]["executableIdentity"]["loader"]["upgradeAuthority"],
        serde_json::json!({"kind": "none"})
    );

    let projection = &vector["projection"];
    let release =
        parse_hosted_managed_program_release_v2(&serde_json::to_vec(projection).unwrap()).unwrap();
    let payload = canonicalize_jcs(projection).unwrap();
    assert_expected_identity(
        &vector["expected"],
        HashKindName::ProgramRelease,
        CanonicalizationProfile::AreteJcsV1,
        &payload,
        release.hash().unwrap().into_any(),
    );
}

#[test]
fn checked_in_decoder_fixture_vectors_are_exact_and_order_invariant() {
    let corpus = corpus();
    assert_eq!(
        corpus["decoderFixtureErrorCategories"],
        serde_json::to_value(DECODER_FIXTURE_ACCOUNT_DECODE_ERROR_CATEGORIES).unwrap()
    );
    assert_eq!(
        [
            DecoderFixtureAccountDecodeErrorCategory::OwnerMismatch.as_str(),
            DecoderFixtureAccountDecodeErrorCategory::UnknownAccountType.as_str(),
            DecoderFixtureAccountDecodeErrorCategory::AccountTypeMismatch.as_str(),
            DecoderFixtureAccountDecodeErrorCategory::AmbiguousAccountType.as_str(),
            DecoderFixtureAccountDecodeErrorCategory::AccountDecodeFailed.as_str(),
        ],
        DECODER_FIXTURE_ACCOUNT_DECODE_ERROR_CATEGORIES
    );
    for vector in corpus["decoderFixtureVectors"]
        .as_array()
        .expect("decoder fixture vectors")
    {
        let fixture: DecoderFixtureSetV2 =
            serde_json::from_value(vector["input"].clone()).expect("fixture projection");
        let projection = fixture.canonical_projection().expect("valid fixture");
        assert_eq!(
            serde_json::to_value(&projection).unwrap(),
            vector["expectedProjection"]
        );
        let payload = canonicalize_jcs(&projection).unwrap();
        assert_expected_identity(
            &vector["expected"],
            HashKindName::DecoderFixtureSet,
            CanonicalizationProfile::AreteJcsV1,
            &payload,
            fixture.hash().unwrap().into_any(),
        );
    }
}

#[test]
fn registry_is_complete_and_domain_separated() {
    let required = [
        "idl-source",
        "idl-content",
        "idl-portable",
        "idl-normalized",
        "program-spec",
        "ast-portable",
        "runtime-artifact",
        "artifact-file",
        "decoder-content",
        "sdk-definition",
        "sdk-extension",
        "sdk-output-tree",
        "compiler",
        "program-release",
        "live-spec",
        "stack-manifest",
        "deployment-release",
        "decoder-fixture-set",
        "knowledge-document",
        "knowledge-snapshot",
        "extension-surface",
        "sdk-install-target",
        "catalog-bundle",
        "catalog-publication-set",
    ];
    let actual: Vec<_> = IDENTITY_REGISTRY
        .iter()
        .map(|metadata| metadata.kind.as_str())
        .collect();
    assert_eq!(actual, required);
    assert_eq!(
        identity_metadata(HashKindName::DecoderContent).visibility,
        Visibility::InternalOnly
    );
    assert_eq!(
        identity_metadata(HashKindName::DecoderFixtureSet).visibility,
        Visibility::InternalOnly
    );
    assert_eq!(
        identity_metadata(HashKindName::DeploymentRelease).visibility,
        Visibility::AuthenticatedOwner
    );
    for metadata in IDENTITY_REGISTRY {
        assert!(!metadata.api_field.is_empty());
        assert!(!metadata.rust_type.is_empty());
        assert!(!metadata.typescript_type.is_empty());
        assert!(!metadata.projection.is_empty());
        assert!(metadata
            .allowed_dto_audiences
            .contains(&metadata.visibility));
    }
    assert_eq!(
        NON_HASH_IDENTITY_REGISTRY
            .iter()
            .map(|metadata| metadata.api_field)
            .collect::<Vec<_>>(),
        [
            "programReadBindingId",
            "decoderBindingId",
            "decoderEngineId"
        ]
    );
    assert_eq!(NON_HASH_IDENTITY_REGISTRY[0].visibility, Visibility::Public);
    assert!(NON_HASH_IDENTITY_REGISTRY[1..]
        .iter()
        .all(|metadata| metadata.visibility == Visibility::InternalOnly));

    let digests: HashSet<_> = IDENTITY_REGISTRY
        .iter()
        .map(|metadata| {
            let preimage = framed_preimage(metadata.kind, metadata.profile, b"same payload");
            Sha256::digest(preimage).to_vec()
        })
        .collect();
    assert_eq!(digests.len(), IDENTITY_REGISTRY.len());
}

#[test]
fn compiler_and_sdk_definition_v1_are_typed_and_order_independent() {
    let corpus = corpus();
    let vectors = &corpus["projectionVectors"];
    let first = CompilerV1::new([
        CompilerSourceV1::new("z.rs", b"z".to_vec()),
        CompilerSourceV1::new("a.rs", b"a".to_vec()),
    ])
    .unwrap();
    let second = CompilerV1::new([
        CompilerSourceV1::new("a.rs", b"a".to_vec()),
        CompilerSourceV1::new("z.rs", b"z".to_vec()),
    ])
    .unwrap();
    let compiler_hash = first.hash().unwrap();
    assert_eq!(compiler_hash, second.hash().unwrap());
    assert_eq!(
        compiler_hash.to_string(),
        vectors["compilerV1"]["expectedHash"]
    );

    let program_spec_hash = HashId::<ProgramSpec>::from_digest([0x11; 32]);
    let definition = SdkDefinitionV1::new(program_spec_hash, compiler_hash);
    assert_eq!(
        serde_json::to_value(&definition).unwrap(),
        vectors["sdkDefinitionV1"]["projection"]
    );
    assert_eq!(
        definition.hash().unwrap().to_string(),
        vectors["sdkDefinitionV1"]["expectedHash"]
    );

    let mut invalid = definition;
    invalid.schema = "arete.sdk-definition/v2".to_string();
    assert_eq!(invalid.hash().unwrap_err().code(), "unknown-version");
}

#[test]
fn typed_projection_hashes_reject_unknown_versions_and_profiles() {
    let corpus = corpus();
    let primary = corpus["idlVectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|vector| vector["id"] == "idl-primary")
        .unwrap();
    let mut spec: ProgramSpecV1 =
        serde_json::from_value(primary["expected"]["programSpec"].clone()).unwrap();
    spec.schema = "arete.program-spec/v2".to_string();
    assert_eq!(spec.hash().unwrap_err().code(), "unknown-version");

    let mut release: OssGeneratedProgramReleaseV1 =
        serde_json::from_value(primary["expected"]["ossRelease"].clone()).unwrap();
    release.release_profile = "hosted-managed".to_string();
    assert_eq!(release.hash().unwrap_err().code(), "invalid-projection");
    release.release_profile = OSS_GENERATED_RELEASE_PROFILE.to_string();
    release.schema = "arete.program-release/v2".to_string();
    assert_eq!(release.hash().unwrap_err().code(), "unknown-version");
}

#[test]
fn typed_hashes_report_known_kind_mismatches() {
    let value = hash_jcs::<ProgramSpec, _>(&serde_json::json!({"safe": true})).unwrap();
    let error = value.to_string().parse::<HashId<IdlContent>>().unwrap_err();
    assert_eq!(error.code(), "unexpected-kind");
}

#[test]
fn release_projections_reject_empty_semantic_identifiers() {
    let corpus = corpus();
    let primary = corpus["idlVectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|vector| vector["id"] == "idl-primary")
        .unwrap();
    let mut oss: OssGeneratedProgramReleaseV1 =
        serde_json::from_value(primary["expected"]["ossRelease"].clone()).unwrap();
    oss.decoder_engine_id.clear();
    assert_eq!(oss.hash().unwrap_err().code(), "invalid-projection");

    let hosted_vector = corpus["releaseVectors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|vector| vector["id"] == "release-hosted-managed-v2-upgradeable")
        .unwrap();
    let mut hosted: HostedManagedProgramReleaseV2 =
        serde_json::from_value(hosted_vector["projection"].clone()).unwrap();
    hosted.decoder_binding_id.clear();
    assert_eq!(hosted.hash().unwrap_err().code(), "invalid-projection");

    let mut hosted: HostedManagedProgramReleaseV2 =
        serde_json::from_value(hosted_vector["projection"].clone()).unwrap();
    hosted.decoder_abi_version.clear();
    assert_eq!(hosted.hash().unwrap_err().code(), "invalid-projection");
}

#[test]
fn decoder_fixture_v2_enforces_case_and_byte_bounds() {
    let corpus = corpus();
    let baseline = &corpus["decoderFixtureVectors"][0]["input"];
    let mut fixture: DecoderFixtureSetV2 = serde_json::from_value(baseline.clone()).unwrap();

    fixture.cases[0].account_data_hex = "00".repeat(DECODER_FIXTURE_MAX_ACCOUNT_BYTES + 1);
    assert_eq!(
        validate_decoder_fixture_set_v2(&fixture)
            .unwrap_err()
            .code(),
        "invalid-projection"
    );

    let mut fixture: DecoderFixtureSetV2 = serde_json::from_value(baseline.clone()).unwrap();
    let template = fixture.cases[0].clone();
    fixture.cases = (0..=DECODER_FIXTURE_MAX_CASES)
        .map(|index| DecoderFixtureCaseV2 {
            id: format!("case-{index}"),
            ..template.clone()
        })
        .collect();
    assert_eq!(
        validate_decoder_fixture_set_v2(&fixture)
            .unwrap_err()
            .code(),
        "invalid-projection"
    );

    let mut fixture: DecoderFixtureSetV2 = serde_json::from_value(baseline.clone()).unwrap();
    let mut template = fixture.cases[0].clone();
    template.account_data_hex = "00".repeat(DECODER_FIXTURE_MAX_ACCOUNT_BYTES);
    fixture.cases = (0..=DECODER_FIXTURE_MAX_TOTAL_ACCOUNT_BYTES
        / DECODER_FIXTURE_MAX_ACCOUNT_BYTES)
        .map(|index| DecoderFixtureCaseV2 {
            id: format!("total-{index}"),
            ..template.clone()
        })
        .collect();
    assert_eq!(
        validate_decoder_fixture_set_v2(&fixture)
            .unwrap_err()
            .code(),
        "invalid-projection"
    );
}

#[test]
fn jcs_rejects_non_finite_serialized_values_and_unsafe_value_integers() {
    assert_eq!(
        canonicalize_jcs(&f64::NAN).unwrap_err().code(),
        "non-finite-number"
    );
    let unsafe_integer = serde_json::json!(9_007_199_254_740_992_u64);
    assert_eq!(
        canonicalize_json_value(&unsafe_integer).unwrap_err().code(),
        "unsafe-json-integer"
    );
}
