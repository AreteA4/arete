//! Regenerate `test-vectors/hash-v1.json`.
//!
//! ```bash
//! cargo run -p arete-hash --example generate_hash_v1_vectors
//! ```

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;

use arete_hash::*;
use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

const OUTPUT: &str = "../test-vectors/hash-v1.json";
const PROGRAM_A: &str = "Demo111111111111111111111111111111111111111";
const PROGRAM_B: &str = "Other11111111111111111111111111111111111111";

fn main() {
    let mut generator = Generator::default();
    generator.hash_id_vectors();
    generator.raw_vectors();
    generator.jcs_vectors();
    generator.domain_separation_vectors();
    generator.tuple_vectors();
    generator.tree_vectors();
    generator.jcs_failure_vectors();
    generator.tuple_tree_failure_vectors();
    generator.self_hash_vectors();
    generator.idl_vectors();
    generator.idl_failure_vectors();
    generator.release_vectors();
    generator.decoder_fixture_vectors();
    generator.finish();
}

#[derive(Default)]
struct Generator {
    hash_id_vectors: Vec<Value>,
    hash_vectors: Vec<Value>,
    failure_vectors: Vec<Value>,
    self_hash_vectors: Vec<Value>,
    idl_vectors: Vec<Value>,
    idl_failure_vectors: Vec<Value>,
    release_vectors: Vec<Value>,
    decoder_fixture_vectors: Vec<Value>,
    hash_ids_by_name: HashMap<String, String>,
    equivalence_groups: BTreeMap<String, Vec<(String, String)>>,
    differs_from: Vec<(String, String)>,
    primary_identity: Option<OssProgramIdentityV1>,
}

impl Generator {
    fn finish(&self) {
        for (group, members) in &self.equivalence_groups {
            let Some((_, first)) = members.first() else {
                continue;
            };
            for (id, hash_id) in members {
                assert_eq!(
                    hash_id, first,
                    "equivalence group '{group}' member '{id}' diverged"
                );
            }
        }
        for (id, other) in &self.differs_from {
            assert_ne!(
                self.hash_ids_by_name.get(id),
                self.hash_ids_by_name.get(other),
                "vector '{id}' must differ from '{other}'"
            );
        }

        let compiler = CompilerV1::new([
            CompilerSourceV1::new("a.rs", b"a".to_vec()),
            CompilerSourceV1::new("z.rs", b"z".to_vec()),
        ])
        .expect("compiler projection");
        let compiler_hash = compiler.hash().expect("compiler hash");
        let sdk_definition = SdkDefinitionV1::new(
            HashId::<ProgramSpec>::from_digest([0x11; 32]),
            compiler_hash,
        );
        let sdk_definition_hash = sdk_definition.hash().expect("SDK definition hash");

        let document = json!({
            "schema": "arete.hash-vectors/v1",
            "protocol": {
                "label": HASH_PROTOCOL_LABEL,
                "version": HASH_PROTOCOL_VERSION,
                "algorithm": HASH_ALGORITHM,
                "typedIdFormat": "arete:h1:<kind>:sha256:<lowercase-hex-digest>",
            },
            "profiles": [
                "raw-bytes-v1",
                "arete-jcs-v1",
                "framed-tuple-v1",
                "artifact-tree-v1",
            ],
            "kinds": IDENTITY_REGISTRY,
            "nonHashIdentities": NON_HASH_IDENTITY_REGISTRY,
            "projectionVectors": {
                "compilerV1": {
                    "projection": {
                        "schema": compiler.schema,
                        "sources": compiler.sources.iter().map(|source| json!({
                            "path": source.path,
                            "bytesHex": hex::encode(&source.bytes),
                        })).collect::<Vec<_>>(),
                    },
                    "expectedHash": compiler_hash,
                },
                "sdkDefinitionV1": {
                    "projection": sdk_definition,
                    "expectedHash": sdk_definition_hash,
                },
            },
            "decoderFixtureErrorCategories": DECODER_FIXTURE_ACCOUNT_DECODE_ERROR_CATEGORIES,
            "normalization": {
                "idlVersion": arete_idl::IDL_NORMALIZATION_VERSION,
                "programSpecSchema": PROGRAM_SPEC_SCHEMA_V1,
                "programReleaseSchema": PROGRAM_RELEASE_SCHEMA_V1,
                "ossDecoderEngineId": OSS_DECODER_ENGINE_ID,
                "decoderFixtureSchema": DECODER_FIXTURE_SCHEMA_V2,
            },
            "inputEncodings": {
                "utf8": "UTF-8 bytes of data; JCS inputs are parsed with duplicate-key and unsafe-integer rejection",
                "hex": "exact bytes encoded as lowercase hexadecimal",
                "tuple": "ordered fields with label and exactly one of valueUtf8 or valueHex",
                "tree": "entries with path, bytesHex, and type (file or symlink)",
            },
            "errorCodes": [
                "invalid-hash-id",
                "unknown-version",
                "unknown-kind",
                "unexpected-kind",
                "unknown-algorithm",
                "profile-mismatch",
                "invalid-json",
                "duplicate-json-key",
                "unsafe-json-integer",
                "non-finite-number",
                "duplicate-tuple-label",
                "invalid-artifact-path",
                "duplicate-artifact-path",
                "symlink-artifact",
                "invalid-self-hash-projection",
                "invalid-projection",
                "missing-program-id",
                "invalid-program-id-location",
                "conflicting-program-ids",
                "invalid-idl",
                "serialization",
            ],
            "hashIdVectors": self.hash_id_vectors,
            "hashVectors": self.hash_vectors,
            "failureVectors": self.failure_vectors,
            "selfHashVectors": self.self_hash_vectors,
            "idlVectors": self.idl_vectors,
            "idlFailureVectors": self.idl_failure_vectors,
            "releaseVectors": self.release_vectors,
            "decoderFixtureVectors": self.decoder_fixture_vectors,
        });

        let mut text = serde_json::to_string_pretty(&document).expect("vectors serialize");
        text.push('\n');
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(OUTPUT);
        fs::write(&path, text).expect("write vectors");
        println!("wrote {}", path.display());
    }

    fn add_hash_vector(
        &mut self,
        id: &str,
        description: &str,
        kind: HashKindName,
        profile: CanonicalizationProfile,
        input: VectorInput,
        input_json: Value,
        projection: Value,
        equivalence_group: Option<&str>,
        differs_from: &[&str],
    ) -> String {
        let outcome = execute_vector(kind, profile, &input)
            .unwrap_or_else(|error| panic!("vector '{id}' unexpectedly failed: {error}"));
        let hash_id = outcome.hash_id.to_string();
        let mut vector = Map::new();
        vector.insert("id".into(), json!(id));
        vector.insert("description".into(), json!(description));
        vector.insert("kind".into(), json!(kind.as_str()));
        vector.insert("profile".into(), json!(profile.as_str()));
        vector.insert("projection".into(), projection);
        vector.insert("input".into(), input_json);
        vector.insert("expected".into(), outcome_json(&outcome));
        if let Some(group) = equivalence_group {
            vector.insert("equivalenceGroup".into(), json!(group));
            self.equivalence_groups
                .entry(group.to_string())
                .or_default()
                .push((id.to_string(), hash_id.clone()));
        }
        if !differs_from.is_empty() {
            vector.insert("differsFrom".into(), json!(differs_from));
            self.differs_from.extend(
                differs_from
                    .iter()
                    .map(|other| (id.to_string(), (*other).to_string())),
            );
        }
        self.hash_ids_by_name
            .insert(id.to_string(), hash_id.clone());
        self.hash_vectors.push(Value::Object(vector));
        hash_id
    }

    fn add_failure(
        &mut self,
        id: &str,
        description: &str,
        operation: &str,
        input: Value,
        expected_error: &HashError,
    ) {
        self.failure_vectors.push(json!({
            "id": id,
            "description": description,
            "operation": operation,
            "input": input,
            "expectedError": expected_error.code(),
        }));
    }

    fn hash_id_vectors(&mut self) {
        let digest = [0xabu8; 32];
        for metadata in IDENTITY_REGISTRY {
            let id = AnyHashId::from_parts(metadata.kind, digest).to_string();
            let parsed = AnyHashId::from_str(&id).expect("generated HashId parses");
            assert_eq!(parsed.kind(), metadata.kind);
            self.hash_id_vectors.push(json!({
                "id": format!("hash-id-valid-{}", metadata.kind),
                "input": id,
                "valid": true,
                "expectedKind": metadata.kind.as_str(),
                "expectedDigestHex": hex::encode(digest),
            }));
        }

        for (id, input, code) in [
            (
                "hash-id-protocol",
                format!("other:h1:idl-content:sha256:{}", hex::encode(digest)),
                "invalid-hash-id",
            ),
            (
                "hash-id-version",
                format!("arete:h2:idl-content:sha256:{}", hex::encode(digest)),
                "unknown-version",
            ),
            (
                "hash-id-kind",
                format!("arete:h1:not-a-kind:sha256:{}", hex::encode(digest)),
                "unknown-kind",
            ),
            (
                "hash-id-algorithm",
                format!("arete:h1:idl-content:sha512:{}", hex::encode(digest)),
                "unknown-algorithm",
            ),
            (
                "hash-id-uppercase",
                format!("arete:h1:idl-content:sha256:{}", hex::encode_upper(digest)),
                "invalid-hash-id",
            ),
            (
                "hash-id-short",
                "arete:h1:idl-content:sha256:ab".to_string(),
                "invalid-hash-id",
            ),
            (
                "hash-id-extra-component",
                format!("arete:h1:idl-content:sha256:{}:extra", hex::encode(digest)),
                "invalid-hash-id",
            ),
        ] {
            let error = AnyHashId::from_str(&input).expect_err("invalid HashId must fail");
            assert_eq!(error.code(), code);
            self.hash_id_vectors.push(json!({
                "id": id,
                "input": input,
                "valid": false,
                "expectedError": code,
            }));
        }
    }

    fn raw_vectors(&mut self) {
        self.add_hash_vector(
            "raw-empty",
            "The empty raw payload is preserved exactly.",
            HashKindName::ArtifactFile,
            CanonicalizationProfile::RawBytesV1,
            VectorInput::RawBytes(Vec::new()),
            json!({"encoding": "utf8", "data": ""}),
            json!({"name": "exact-bytes"}),
            None,
            &[],
        );
        self.add_hash_vector(
            "raw-lf",
            "LF bytes are not normalized.",
            HashKindName::ArtifactFile,
            CanonicalizationProfile::RawBytesV1,
            VectorInput::RawBytes(b"line one\nline two\n".to_vec()),
            json!({"encoding": "utf8", "data": "line one\nline two\n"}),
            json!({"name": "exact-bytes", "lineEnding": "lf"}),
            None,
            &["raw-crlf"],
        );
        self.add_hash_vector(
            "raw-crlf",
            "CRLF bytes are not normalized.",
            HashKindName::ArtifactFile,
            CanonicalizationProfile::RawBytesV1,
            VectorInput::RawBytes(b"line one\r\nline two\r\n".to_vec()),
            json!({"encoding": "utf8", "data": "line one\r\nline two\r\n"}),
            json!({"name": "exact-bytes", "lineEnding": "crlf"}),
            None,
            &[],
        );
        self.add_hash_vector(
            "raw-lf-idl-source",
            "The same bytes under idl-source prove kind domain separation.",
            HashKindName::IdlSource,
            CanonicalizationProfile::RawBytesV1,
            VectorInput::RawBytes(b"line one\nline two\n".to_vec()),
            json!({"encoding": "utf8", "data": "line one\nline two\n"}),
            json!({"name": "exact-source"}),
            None,
            &["raw-lf"],
        );
        self.add_hash_vector(
            "raw-non-utf8",
            "Raw payloads may contain malformed UTF-8.",
            HashKindName::ArtifactFile,
            CanonicalizationProfile::RawBytesV1,
            VectorInput::RawBytes(vec![0xff, 0x00, 0xfe, 0x0a]),
            json!({"encoding": "hex", "data": "ff00fe0a"}),
            json!({"name": "exact-bytes"}),
            None,
            &[],
        );
    }

    fn jcs_vectors(&mut self) {
        let cases = [
            (
                "jcs-key-order-a",
                "Recursive object key ordering, first spelling.",
                r#"{"b":1,"a":{"d":4,"c":3},"arr":[1,2,3]}"#,
                Some("jcs-key-order"),
                &[][..],
            ),
            (
                "jcs-key-order-b",
                "Recursive object key ordering, second spelling.",
                r#"{"arr":[1,2,3],"a":{"c":3,"d":4},"b":1}"#,
                Some("jcs-key-order"),
                &[][..],
            ),
            (
                "jcs-array-order-a",
                "Array order is significant.",
                r#"{"arr":[1,2,3]}"#,
                None,
                &["jcs-array-order-b"][..],
            ),
            (
                "jcs-array-order-b",
                "Reordered array elements change identity.",
                r#"{"arr":[3,2,1]}"#,
                None,
                &[][..],
            ),
            (
                "jcs-unicode-key-escaped",
                "An escaped Unicode key.",
                r#"{"\u00e9":1,"z":2}"#,
                Some("jcs-unicode-key"),
                &[][..],
            ),
            (
                "jcs-unicode-key-utf8",
                "The same Unicode key as raw UTF-8.",
                r#"{"é":1,"z":2}"#,
                Some("jcs-unicode-key"),
                &[][..],
            ),
            (
                "jcs-escaped-string-a",
                "Short escapes and an unescaped solidus.",
                r#"{"s":"a\"b\\c/d\be\ff\ng\rh\ti"}"#,
                Some("jcs-escaped-string"),
                &[][..],
            ),
            (
                "jcs-escaped-string-b",
                "An escaped solidus has the same value.",
                r#"{"s":"a\"b\\c\/d\be\ff\ng\rh\ti"}"#,
                Some("jcs-escaped-string"),
                &[][..],
            ),
            (
                "jcs-null",
                "Explicit null is part of the projection.",
                r#"{"a":null,"b":1}"#,
                None,
                &["jcs-absent"][..],
            ),
            (
                "jcs-absent",
                "Absence remains distinct from null.",
                r#"{"b":1}"#,
                None,
                &[][..],
            ),
            (
                "jcs-int-safe-max",
                "Maximum safe integer token.",
                r#"{"n":9007199254740991}"#,
                None,
                &[][..],
            ),
            (
                "jcs-int-safe-min",
                "Minimum safe integer token.",
                r#"{"n":-9007199254740991}"#,
                None,
                &[][..],
            ),
            (
                "jcs-unsafe-integer-as-string",
                "Unsafe integer values are valid when represented as strings.",
                r#"{"n":"9007199254740992"}"#,
                None,
                &[][..],
            ),
            (
                "jcs-number-fraction",
                "RFC 8785 fraction serialization.",
                r#"{"n":0.1}"#,
                None,
                &[][..],
            ),
            (
                "jcs-number-exponent",
                "Exponent form canonicalizes to the represented double.",
                r#"{"n":1e2}"#,
                Some("jcs-number-hundred"),
                &[][..],
            ),
            (
                "jcs-number-hundred",
                "Decimal spelling of the same double.",
                r#"{"n":100.0}"#,
                Some("jcs-number-hundred"),
                &[][..],
            ),
            (
                "jcs-number-small-exponent",
                "Values below 1e-6 use exponential notation.",
                r#"{"n":1.5e-7}"#,
                None,
                &[][..],
            ),
            (
                "jcs-number-decimal-low-boundary",
                "1e-6 uses decimal notation.",
                r#"{"n":1e-6}"#,
                None,
                &[][..],
            ),
            (
                "jcs-number-exp-high-boundary",
                "1e21 uses exponential notation.",
                r#"{"n":1e21}"#,
                None,
                &[][..],
            ),
            (
                "jcs-number-decimal-high-boundary",
                "1e20 uses decimal notation.",
                r#"{"n":1e20}"#,
                None,
                &[][..],
            ),
            (
                "jcs-number-negative-zero",
                "Negative zero canonicalizes to zero.",
                r#"{"n":-0.0}"#,
                Some("jcs-number-zero"),
                &[][..],
            ),
            (
                "jcs-number-zero",
                "Integer zero has the same canonical double spelling.",
                r#"{"n":0}"#,
                Some("jcs-number-zero"),
                &[][..],
            ),
            (
                "jcs-number-precision",
                "ECMAScript round-trip precision.",
                r#"{"n":333333333.33333329}"#,
                None,
                &[][..],
            ),
            (
                "jcs-number-large-double",
                "Large fractional/exponent numbers follow RFC 8785.",
                r#"{"n":1.2345678901234568e29}"#,
                None,
                &[][..],
            ),
            (
                "jcs-exponent-mathematical-integer",
                "The safe-integer restriction applies to integer tokens, not exponent forms.",
                r#"{"n":9007199254740992e0}"#,
                None,
                &[][..],
            ),
            (
                "jcs-nested",
                "Nested objects, arrays, booleans, strings, and null.",
                r#"{"z":[{"b":2,"a":1}],"a":{"y":[true,null,"x"]}}"#,
                None,
                &[][..],
            ),
        ];

        for (id, description, data, group, differs) in cases {
            self.add_hash_vector(
                id,
                description,
                HashKindName::IdlContent,
                CanonicalizationProfile::AreteJcsV1,
                VectorInput::RawBytes(data.as_bytes().to_vec()),
                json!({"encoding": "utf8", "data": data}),
                json!({"name": "rfc8785-json", "unsafeIntegerPolicy": "integer-token"}),
                group,
                differs,
            );
        }

        self.add_hash_vector(
            "jcs-utf16-key-sort",
            "JCS sorts object keys by UTF-16 code units; U+10000 precedes U+FFFD.",
            HashKindName::IdlContent,
            CanonicalizationProfile::AreteJcsV1,
            VectorInput::RawBytes("{\"�\":1,\"𐀀\":2}".as_bytes().to_vec()),
            json!({"encoding": "utf8", "data": "{\"�\":1,\"𐀀\":2}"}),
            json!({"name": "rfc8785-json", "keyOrder": "utf16-code-unit"}),
            None,
            &[],
        );
    }

    fn domain_separation_vectors(&mut self) {
        for (id, kind) in [
            ("domain-idl-content", HashKindName::IdlContent),
            ("domain-ast-portable", HashKindName::AstPortable),
            ("domain-program-spec", HashKindName::ProgramSpec),
        ] {
            self.add_hash_vector(
                id,
                "The same canonical JSON is separated by hash kind.",
                kind,
                CanonicalizationProfile::AreteJcsV1,
                VectorInput::RawBytes(br#"{"value":1}"#.to_vec()),
                json!({"encoding": "utf8", "data": "{\"value\":1}"}),
                json!({"name": "domain-separation"}),
                None,
                match id {
                    "domain-ast-portable" => &["domain-idl-content"],
                    "domain-program-spec" => &["domain-idl-content", "domain-ast-portable"],
                    _ => &[],
                },
            );
        }
    }

    fn tuple_vectors(&mut self) {
        let idl_content = self.hash_ids_by_name["domain-idl-content"].clone();
        let artifact_file = self.hash_ids_by_name["raw-lf"].clone();
        self.add_hash_vector(
            "tuple-compiler-composite",
            "HashId fields use their complete canonical textual form.",
            HashKindName::Compiler,
            CanonicalizationProfile::FramedTupleV1,
            VectorInput::TupleFields(vec![
                ("idlContent".into(), idl_content.clone().into_bytes()),
                ("sdkGenerator".into(), artifact_file.clone().into_bytes()),
            ]),
            json!({
                "encoding": "tuple",
                "fields": [
                    {"label": "idlContent", "valueUtf8": idl_content},
                    {"label": "sdkGenerator", "valueUtf8": artifact_file},
                ]
            }),
            json!({"name": "ordered-composite", "hashIdEncoding": "canonical-text"}),
            None,
            &[],
        );
        self.add_hash_vector(
            "tuple-binary-field",
            "Tuple values are arbitrary exact bytes.",
            HashKindName::Compiler,
            CanonicalizationProfile::FramedTupleV1,
            VectorInput::TupleFields(vec![("only".into(), vec![0x00, 0xff])]),
            json!({"encoding": "tuple", "fields": [{"label": "only", "valueHex": "00ff"}]}),
            json!({"name": "ordered-composite"}),
            None,
            &[],
        );
    }

    fn tree_vectors(&mut self) {
        self.add_hash_vector(
            "tree-empty",
            "The empty tree payload is eight zero bytes.",
            HashKindName::SdkOutputTree,
            CanonicalizationProfile::ArtifactTreeV1,
            VectorInput::TreeEntries(vec![]),
            json!({"encoding": "tree", "entries": []}),
            json!({"name": "artifact-tree", "pathOrder": "raw-utf8"}),
            None,
            &[],
        );
        self.add_hash_vector(
            "tree-single",
            "A single exact file leaf.",
            HashKindName::SdkOutputTree,
            CanonicalizationProfile::ArtifactTreeV1,
            VectorInput::TreeEntries(vec![VectorTreeEntry {
                path: "index.ts".into(),
                bytes: b"export {}\n".to_vec(),
                symlink: false,
            }]),
            json!({"encoding": "tree", "entries": [{"path": "index.ts", "bytesHex": "6578706f7274207b7d0a", "type": "file"}]}),
            json!({"name": "artifact-tree", "leafKind": "artifact-file"}),
            None,
            &[],
        );

        let entries = [
            VectorTreeEntry {
                path: "package.json".into(),
                bytes: b"{}\n".to_vec(),
                symlink: false,
            },
            VectorTreeEntry {
                path: "src/core.ts".into(),
                bytes: b"// core\n".to_vec(),
                symlink: false,
            },
            VectorTreeEntry {
                path: "src/index.ts".into(),
                bytes: b"// index\n".to_vec(),
                symlink: false,
            },
        ];
        for (id, order) in [("tree-multi-a", [0, 1, 2]), ("tree-multi-b", [2, 1, 0])] {
            let ordered: Vec<_> = order.iter().map(|&index| entries[index].clone()).collect();
            let input_json = json!({
                "encoding": "tree",
                "entries": ordered.iter().map(|entry| json!({
                    "path": entry.path,
                    "bytesHex": hex::encode(&entry.bytes),
                    "type": "file",
                })).collect::<Vec<_>>()
            });
            self.add_hash_vector(
                id,
                "Tree insertion order does not change its identity.",
                HashKindName::SdkOutputTree,
                CanonicalizationProfile::ArtifactTreeV1,
                VectorInput::TreeEntries(ordered),
                input_json,
                json!({"name": "artifact-tree", "pathOrder": "raw-utf8"}),
                Some("tree-insertion-order"),
                &[],
            );
        }
        self.add_hash_vector(
            "tree-unicode-path",
            "Path Unicode bytes are not normalized.",
            HashKindName::SdkOutputTree,
            CanonicalizationProfile::ArtifactTreeV1,
            VectorInput::TreeEntries(vec![VectorTreeEntry {
                path: "src/é.ts".into(),
                bytes: b"x".to_vec(),
                symlink: false,
            }]),
            json!({"encoding": "tree", "entries": [{"path": "src/é.ts", "bytesHex": "78", "type": "file"}]}),
            json!({"name": "artifact-tree", "unicodeNormalization": "none"}),
            None,
            &[],
        );
    }

    fn jcs_failure_vectors(&mut self) {
        for (id, description, bytes, code) in [
            (
                "jcs-duplicate-key",
                "Duplicate keys are rejected before canonicalization.",
                br#"{"a":1,"a":2}"#.to_vec(),
                "duplicate-json-key",
            ),
            (
                "jcs-duplicate-key-nested",
                "Nested duplicate keys are also rejected.",
                br#"{"x":{"a":1,"a":2}}"#.to_vec(),
                "duplicate-json-key",
            ),
            (
                "jcs-unsafe-positive-integer",
                "An integer token above the inclusive safe range is rejected.",
                br#"{"n":9007199254740992}"#.to_vec(),
                "unsafe-json-integer",
            ),
            (
                "jcs-unsafe-negative-integer",
                "An integer token below the inclusive safe range is rejected.",
                br#"{"n":-9007199254740992}"#.to_vec(),
                "unsafe-json-integer",
            ),
            (
                "jcs-malformed-utf8",
                "JCS input must be valid UTF-8 JSON.",
                vec![b'{', b'"', b'x', b'"', b':', b'"', 0xff, b'"', b'}'],
                "invalid-json",
            ),
            (
                "jcs-invalid-syntax",
                "Malformed JSON is rejected.",
                br#"{"a":}"#.to_vec(),
                "invalid-json",
            ),
            (
                "jcs-non-finite-overflow",
                "A numeric token that overflows IEEE-754 is non-finite.",
                br#"{"n":1e400}"#.to_vec(),
                "non-finite-number",
            ),
        ] {
            let error = canonicalize_json_bytes(&bytes).expect_err("invalid JCS input must fail");
            assert_eq!(error.code(), code);
            self.add_failure(
                id,
                description,
                "arete-jcs-v1",
                json!({"encoding": "hex", "data": hex::encode(bytes)}),
                &error,
            );
        }

        let mismatch = execute_vector(
            HashKindName::IdlContent,
            CanonicalizationProfile::RawBytesV1,
            &VectorInput::RawBytes(b"{}".to_vec()),
        )
        .expect_err("profile mismatch must fail");
        self.add_failure(
            "profile-kind-mismatch",
            "Kinds have one required canonicalization profile.",
            "execute-vector",
            json!({
                "kind": "idl-content",
                "profile": "raw-bytes-v1",
                "input": {"encoding": "utf8", "data": "{}"}
            }),
            &mismatch,
        );

        let unknown_profile = CanonicalizationProfile::from_str("arete-jcs-v2")
            .expect_err("unknown profile must fail");
        self.add_failure(
            "profile-unknown-version",
            "Unknown canonicalization profile versions fail closed.",
            "parse-profile",
            json!({"profile": "arete-jcs-v2"}),
            &unknown_profile,
        );
    }

    fn tuple_tree_failure_vectors(&mut self) {
        let duplicate =
            framed_tuple_payload(&[TupleField::new("same", b"a"), TupleField::new("same", b"b")])
                .expect_err("duplicate tuple label must fail");
        self.add_failure(
            "tuple-duplicate-label",
            "Tuple labels are unique.",
            "framed-tuple-v1",
            json!({"encoding": "tuple", "fields": [
                {"label": "same", "valueUtf8": "a"},
                {"label": "same", "valueUtf8": "b"}
            ]}),
            &duplicate,
        );

        for (id, path) in [
            ("tree-path-empty", ""),
            ("tree-path-leading-slash", "/a"),
            ("tree-path-trailing-slash", "a/"),
            ("tree-path-repeated-slash", "a//b"),
            ("tree-path-dot", "a/./b"),
            ("tree-path-dotdot", "a/../b"),
            ("tree-path-backslash", "a\\b"),
            ("tree-path-nul", "a\0b"),
        ] {
            let error = artifact_tree_payload(&[ArtifactTreeEntry::file(path, b"x")])
                .expect_err("invalid path must fail");
            self.add_failure(
                id,
                "Artifact paths must be canonical POSIX relative paths.",
                "artifact-tree-v1",
                json!({"encoding": "tree", "entries": [{"path": path, "bytesHex": "78", "type": "file"}]}),
                &error,
            );
        }

        let duplicate_path = artifact_tree_payload(&[
            ArtifactTreeEntry::file("a", b"x"),
            ArtifactTreeEntry::file("a", b"y"),
        ])
        .expect_err("duplicate path must fail");
        self.add_failure(
            "tree-duplicate-path",
            "Duplicate paths are rejected.",
            "artifact-tree-v1",
            json!({"encoding": "tree", "entries": [
                {"path": "a", "bytesHex": "78", "type": "file"},
                {"path": "a", "bytesHex": "79", "type": "file"}
            ]}),
            &duplicate_path,
        );

        let symlink = artifact_tree_payload(&[ArtifactTreeEntry::symlink("link")])
            .expect_err("symlink must fail");
        self.add_failure(
            "tree-symlink",
            "Symlink entries are rejected.",
            "artifact-tree-v1",
            json!({"encoding": "tree", "entries": [{"path": "link", "bytesHex": "", "type": "symlink"}]}),
            &symlink,
        );
    }

    fn self_hash_vectors(&mut self) {
        let with_self_hash = json!({
            "schema": "arete.portable-ast/v1",
            "artifactHash": "remove-only-this-value",
            "nested": {"artifactHash": "preserve-nested"},
            "otherHash": "preserve-other-hash"
        });
        let without_self_hash = json!({
            "schema": "arete.portable-ast/v1",
            "nested": {"artifactHash": "preserve-nested"},
            "otherHash": "preserve-other-hash"
        });
        let mut expected_hash = None;
        for (id, input, group) in [
            (
                "self-hash-present",
                with_self_hash,
                "portable-ast-self-hash",
            ),
            (
                "self-hash-absent",
                without_self_hash,
                "portable-ast-self-hash",
            ),
        ] {
            let projection = project_without_artifact_hash(&input).expect("object projection");
            let payload = canonicalize_jcs(&projection).expect("projection canonicalizes");
            let hash = hash_jcs::<AstPortable, _>(&projection).expect("projection hashes");
            if let Some(previous) = expected_hash.as_ref() {
                assert_eq!(previous, &hash.to_string());
            }
            expected_hash = Some(hash.to_string());
            self.self_hash_vectors.push(json!({
                "id": id,
                "description": "Only the declared top-level artifactHash is removed.",
                "kind": "ast-portable",
                "profile": "arete-jcs-v1",
                "projection": {"name": "portable-ast-v1", "removedField": "artifactHash", "scope": "top-level-only"},
                "equivalenceGroup": group,
                "input": input,
                "expectedProjection": projection,
                "expected": expectation(HashKindName::AstPortable, CanonicalizationProfile::AreteJcsV1, &payload, hash.into_any()),
            }));
        }

        let error = project_without_artifact_hash(&json!([1, 2]))
            .expect_err("non-object self-hash input must fail");
        self.add_failure(
            "self-hash-non-object",
            "Self-hash removal applies only to declared object projections.",
            "remove-artifact-hash",
            json!({"value": [1, 2]}),
            &error,
        );
    }

    fn idl_vectors(&mut self) {
        let primary = sample_idl(PROGRAM_A);
        let address_variant = sample_idl(PROGRAM_B);
        let reordered = sample_idl_reordered(PROGRAM_A);
        let crlf = primary.replace('\n', "\r\n");
        let explicit = sample_idl_without_program_id();

        let primary_document =
            CanonicalIdlDocument::parse(primary.as_bytes(), None).expect("primary IDL parses");
        let primary_identity = OssProgramIdentityV1::from_document(&primary_document)
            .expect("primary identity derives");
        let primary_portable = primary_document.hashes().portable.to_string();
        let primary_content = primary_document.hashes().content.to_string();

        for (id, source, explicit_program_id, metadata) in [
            (
                "idl-primary",
                primary,
                None,
                json!({
                    "producers": ["macro", "interpreter", "cli"],
                    "portableEquivalenceGroup": "idl-address-removal",
                    "contentEquivalenceGroup": "idl-semantic-json",
                }),
            ),
            (
                "idl-address-variant",
                address_variant,
                None,
                json!({"portableEquivalenceGroup": "idl-address-removal"}),
            ),
            (
                "idl-key-order-variant",
                reordered,
                None,
                json!({"contentEquivalenceGroup": "idl-semantic-json"}),
            ),
            (
                "idl-whitespace-crlf-variant",
                crlf,
                None,
                json!({"contentEquivalenceGroup": "idl-semantic-json", "sourceLineEnding": "crlf"}),
            ),
            (
                "idl-explicit-program-id",
                explicit,
                Some(PROGRAM_A),
                json!({"programIdSource": "explicit"}),
            ),
        ] {
            let document = CanonicalIdlDocument::parse(source.as_bytes(), explicit_program_id)
                .unwrap_or_else(|error| panic!("IDL vector '{id}' failed: {error}"));
            let identity = OssProgramIdentityV1::from_document(&document)
                .unwrap_or_else(|error| panic!("IDL vector '{id}' identity failed: {error}"));
            let vector = idl_vector_json(
                id,
                &source,
                explicit_program_id,
                metadata,
                &document,
                &identity,
            );

            if matches!(id, "idl-address-variant") {
                assert_eq!(document.hashes().portable.to_string(), primary_portable);
                assert_ne!(document.hashes().content.to_string(), primary_content);
            }
            if matches!(id, "idl-key-order-variant" | "idl-whitespace-crlf-variant") {
                assert_eq!(document.hashes().content.to_string(), primary_content);
                assert_ne!(document.hashes().source, primary_document.hashes().source);
            }
            self.idl_vectors.push(vector);
        }
        self.primary_identity = Some(primary_identity);
    }

    fn idl_failure_vectors(&mut self) {
        let cases = [
            (
                "idl-missing-program-id",
                minimal_idl_fields(""),
                None,
                "missing-program-id",
            ),
            (
                "idl-conflicting-source-program-ids",
                format!(
                    "{{\"address\":\"{PROGRAM_A}\",\"metadata\":{{\"address\":\"{PROGRAM_B}\"}},{}}}",
                    minimal_idl_fields_body()
                ),
                None,
                "conflicting-program-ids",
            ),
            (
                "idl-conflicting-explicit-program-id",
                minimal_idl_fields(PROGRAM_A),
                Some(PROGRAM_B),
                "conflicting-program-ids",
            ),
            (
                "idl-non-string-address",
                format!("{{\"address\":7,{}}}", minimal_idl_fields_body()),
                None,
                "invalid-program-id-location",
            ),
            (
                "idl-non-object-metadata",
                format!(
                    "{{\"address\":\"{PROGRAM_A}\",\"metadata\":7,{}}}",
                    minimal_idl_fields_body()
                ),
                None,
                "invalid-program-id-location",
            ),
            (
                "idl-duplicate-key",
                format!(
                    "{{\"address\":\"{PROGRAM_A}\",\"address\":\"{PROGRAM_A}\",{}}}",
                    minimal_idl_fields_body()
                ),
                None,
                "duplicate-json-key",
            ),
            (
                "idl-unsafe-integer",
                format!(
                    "{{\"address\":\"{PROGRAM_A}\",\"unknown\":9007199254740992,{}}}",
                    minimal_idl_fields_body()
                ),
                None,
                "unsafe-json-integer",
            ),
        ];

        for (id, source, explicit_program_id, code) in cases {
            let error = CanonicalIdlDocument::parse(source.as_bytes(), explicit_program_id)
                .expect_err("invalid IDL vector must fail");
            assert_eq!(error.code(), code, "IDL failure vector '{id}'");
            self.idl_failure_vectors.push(json!({
                "id": id,
                "input": {
                    "encoding": "utf8",
                    "data": source,
                    "explicitProgramId": explicit_program_id,
                },
                "expectedError": code,
            }));
        }
    }

    fn release_vectors(&mut self) {
        let identity = self
            .primary_identity
            .clone()
            .expect("IDL vectors run before release vectors");
        let baseline = identity.release.clone();

        let mut unknown_spec_schema = identity.program_spec.clone();
        unknown_spec_schema.schema = "arete.program-spec/v2".to_string();
        let error = unknown_spec_schema
            .hash()
            .expect_err("unknown ProgramSpec schema must fail");
        self.add_failure(
            "program-spec-unknown-schema",
            "Unknown ProgramSpec schema versions fail closed.",
            "program-spec-v1",
            json!({"projection": unknown_spec_schema}),
            &error,
        );

        let mut unknown_normalization = identity.program_spec.clone();
        unknown_normalization.idl_snapshot.normalization_version = 2;
        let error = unknown_normalization
            .hash()
            .expect_err("unknown normalization version must fail");
        let mut valid_projection = unknown_normalization.clone();
        valid_projection.idl_snapshot.normalization_version = arete_idl::IDL_NORMALIZATION_VERSION;
        self.add_failure(
            "program-spec-unknown-normalization-version",
            "Unknown IDL normalization versions fail before ProgramSpec hashing.",
            "program-spec-v1",
            json!({
                "projection": valid_projection,
                "normalizationVersionOverride": 2
            }),
            &error,
        );

        let mut unknown_release_schema = baseline.clone();
        unknown_release_schema.schema = "arete.program-release/v2".to_string();
        let error = unknown_release_schema
            .hash()
            .expect_err("unknown release schema must fail");
        self.add_failure(
            "program-release-unknown-schema",
            "Unknown program release schema versions fail closed.",
            "oss-program-release-v1",
            json!({"projection": unknown_release_schema}),
            &error,
        );

        let mut unknown_release_profile = baseline.clone();
        unknown_release_profile.release_profile = "custom".to_string();
        let error = unknown_release_profile
            .hash()
            .expect_err("unknown release profile must fail");
        self.add_failure(
            "program-release-unknown-profile",
            "Unknown OSS release profiles fail closed.",
            "oss-program-release-v1",
            json!({"projection": unknown_release_profile}),
            &error,
        );

        self.add_release_vector(
            "release-oss-macro-cli",
            "The macro, interpreter, and direct CLI generator share this OSS release projection.",
            &baseline,
            json!({
                "producers": ["macro", "interpreter", "cli"],
                "customDecoderDescriptorRequired": false,
                "unrelatedPackageVersion": "0.3.0"
            }),
            Some("oss-package-version-invariant"),
            &[],
        );
        self.add_release_vector(
            "release-oss-unrelated-package-version",
            "An unrelated package version is metadata outside the projection.",
            &baseline,
            json!({"unrelatedPackageVersion": "99.0.0"}),
            Some("oss-package-version-invariant"),
            &[],
        );

        let engine_variant = OssGeneratedProgramReleaseV1::with_decoder_engine(
            baseline.program_id.clone(),
            baseline.program_spec_hash,
            baseline.idl_content_hash,
            baseline.normalized_idl_hash,
            "arete-oss-generated-decoder/v2",
        );
        self.add_release_vector(
            "release-oss-decoder-engine-change",
            "Changing only decoderEngineId changes the release.",
            &engine_variant,
            json!({"changedField": "decoderEngineId"}),
            None,
            &["release-oss-macro-cli"],
        );

        let alternate_spec_hash =
            hash_jcs::<ProgramSpec, _>(&json!({"variant": "program-spec"})).expect("variant hash");
        let spec_variant = OssGeneratedProgramReleaseV1::new(
            baseline.program_id.clone(),
            alternate_spec_hash,
            baseline.idl_content_hash,
            baseline.normalized_idl_hash,
        );
        self.add_release_vector(
            "release-oss-program-spec-change",
            "Changing only programSpecHash changes the release.",
            &spec_variant,
            json!({"changedField": "programSpecHash"}),
            None,
            &["release-oss-macro-cli"],
        );

        let alternate_idl_hash =
            hash_jcs::<IdlContent, _>(&json!({"variant": "idl"})).expect("variant hash");
        let idl_variant = OssGeneratedProgramReleaseV1::new(
            baseline.program_id.clone(),
            baseline.program_spec_hash,
            alternate_idl_hash,
            baseline.normalized_idl_hash,
        );
        self.add_release_vector(
            "release-oss-idl-change",
            "Changing only idlContentHash changes the release.",
            &idl_variant,
            json!({"changedField": "idlContentHash"}),
            None,
            &["release-oss-macro-cli"],
        );

        let hosted = json!({
            "schema": PROGRAM_RELEASE_SCHEMA_V1,
            "releaseProfile": HOSTED_MANAGED_RELEASE_PROFILE,
            "programId": baseline.program_id,
            "programSpecHash": baseline.program_spec_hash,
            "idlContentHash": baseline.idl_content_hash,
            "normalizedIdlHash": baseline.normalized_idl_hash,
            "decoderAbiVersion": "1",
            "decoderEngineId": "arete-hosted-decoder-engine/v1",
            "decoderBindingId": "dec_AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcY",
        });
        self.add_release_vector(
            "release-hosted-managed",
            "Exact hosted internal projection includes immutable random binding and engine IDs.",
            &hosted,
            json!({
                "visibility": "internal-projection",
                "publicOutput": "final-program-release-hash-only"
            }),
            None,
            &[],
        );

        let upgradeable_identity = SolanaExecutableIdentityV1::new(
            "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
            SolanaExecutableLoaderV1::bpf_upgradeable_loader(
                "So11111111111111111111111111111111111111112",
                9_007_199_254_740_993,
                SolanaUpgradeAuthorityV1::address("oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv")
                    .expect("upgrade authority"),
                format!("sha256:{}", "ab".repeat(32)),
            )
            .expect("upgradeable loader identity"),
        )
        .expect("executable identity");
        let hosted_upgradeable =
            HostedManagedProgramReleaseV2::new(HostedManagedProgramReleaseV2Fields {
                program_id: "oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv".to_string(),
                program_spec_hash: baseline.program_spec_hash,
                idl_content_hash: baseline.idl_content_hash,
                normalized_idl_hash: baseline.normalized_idl_hash,
                decoder_abi_version: "1".to_string(),
                decoder_engine_id: "arete-hosted-decoder-engine/v1".to_string(),
                decoder_binding_id: "dec_AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcY".to_string(),
                executable_identity: upgradeable_identity,
            })
            .expect("hosted upgradeable release");
        self.add_release_vector(
            "release-hosted-managed-v2-upgradeable",
            "Hosted V2 binds an upgradeable executable without lossy deployment-slot numbers.",
            &hosted_upgradeable,
            json!({"loader": "bpf-upgradeable-loader"}),
            None,
            &[],
        );

        let no_authority_identity = SolanaExecutableIdentityV1::new(
            "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
            SolanaExecutableLoaderV1::bpf_upgradeable_loader(
                "So11111111111111111111111111111111111111112",
                9_007_199_254_740_993,
                SolanaUpgradeAuthorityV1::none(),
                format!("sha256:{}", "ab".repeat(32)),
            )
            .expect("upgradeable loader identity without authority"),
        )
        .expect("executable identity without authority");
        let hosted_no_authority =
            HostedManagedProgramReleaseV2::new(HostedManagedProgramReleaseV2Fields {
                program_id: "oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv".to_string(),
                program_spec_hash: baseline.program_spec_hash,
                idl_content_hash: baseline.idl_content_hash,
                normalized_idl_hash: baseline.normalized_idl_hash,
                decoder_abi_version: "1".to_string(),
                decoder_engine_id: "arete-hosted-decoder-engine/v1".to_string(),
                decoder_binding_id: "dec_AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcY".to_string(),
                executable_identity: no_authority_identity,
            })
            .expect("hosted release without upgrade authority");
        self.add_release_vector(
            "release-hosted-managed-v2-upgradeable-no-authority",
            "Hosted V2 canonically represents a permanently immutable upgradeable executable.",
            &hosted_no_authority,
            json!({
                "loader": "bpf-upgradeable-loader",
                "upgradeAuthority": "none"
            }),
            None,
            &[],
        );

        let legacy_identity = SolanaExecutableIdentityV1::new(
            "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
            SolanaExecutableLoaderV1::bpf_loader_v2(format!("sha256:{}", "cd".repeat(32)))
                .expect("legacy loader identity"),
        )
        .expect("executable identity");
        let hosted_legacy =
            HostedManagedProgramReleaseV2::new(HostedManagedProgramReleaseV2Fields {
                program_id: "oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv".to_string(),
                program_spec_hash: baseline.program_spec_hash,
                idl_content_hash: baseline.idl_content_hash,
                normalized_idl_hash: baseline.normalized_idl_hash,
                decoder_abi_version: "1".to_string(),
                decoder_engine_id: "arete-hosted-decoder-engine/v1".to_string(),
                decoder_binding_id: "dec_AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcY".to_string(),
                executable_identity: legacy_identity,
            })
            .expect("hosted legacy release");
        self.add_release_vector(
            "release-hosted-managed-v2-legacy-loader",
            "Hosted V2 binds an immutable legacy BPF loader executable.",
            &hosted_legacy,
            json!({"loader": "bpf-loader-v2"}),
            None,
            &[],
        );

        self.hosted_release_failure_vectors(&hosted_upgradeable, &hosted_legacy);
    }

    fn hosted_release_failure_vectors(
        &mut self,
        upgradeable: &HostedManagedProgramReleaseV2,
        legacy: &HostedManagedProgramReleaseV2,
    ) {
        let upgradeable_value = serde_json::to_value(upgradeable).expect("release serializes");
        let legacy_value = serde_json::to_value(legacy).expect("release serializes");
        for (id, description, use_legacy, mutation, expected_code) in [
            (
                "hosted-release-v2-rejects-v1-schema",
                "Hosted managed release V1 is not accepted by the primary V2 API.",
                false,
                "schema",
                "unknown-version",
            ),
            (
                "hosted-release-v2-rejects-unknown-field",
                "Hosted V2 rejects unknown release fields.",
                false,
                "release-field",
                "invalid-projection",
            ),
            (
                "hosted-release-v2-rejects-invalid-program-id",
                "Hosted V2 requires a Solana program public key.",
                false,
                "program-id",
                "invalid-projection",
            ),
            (
                "executable-identity-rejects-invalid-genesis",
                "Executable identity requires a canonical 32-byte base58 genesis hash.",
                false,
                "genesis",
                "invalid-projection",
            ),
            (
                "executable-identity-rejects-leading-zero-genesis",
                "Executable identity rejects a Base58 value with an extra leading zero byte.",
                false,
                "genesis-leading-zero",
                "invalid-projection",
            ),
            (
                "executable-identity-rejects-unknown-field",
                "Executable identity rejects unknown fields.",
                false,
                "identity-field",
                "invalid-projection",
            ),
            (
                "executable-identity-rejects-unknown-loader",
                "Executable identity rejects future loader variants until explicitly versioned.",
                false,
                "loader-kind",
                "invalid-projection",
            ),
            (
                "executable-identity-rejects-loader-id-mismatch",
                "Each loader variant requires its fixed Solana loader program ID.",
                false,
                "loader-id",
                "invalid-projection",
            ),
            (
                "executable-identity-rejects-invalid-program-data",
                "Upgradeable identities require a ProgramData public key.",
                false,
                "program-data",
                "invalid-projection",
            ),
            (
                "executable-identity-rejects-missing-deployment-slot",
                "Upgradeable identities require a deployment slot.",
                false,
                "missing-slot",
                "invalid-projection",
            ),
            (
                "executable-identity-rejects-numeric-deployment-slot",
                "Deployment slots are strings so JavaScript cannot lose precision.",
                false,
                "numeric-slot",
                "invalid-projection",
            ),
            (
                "executable-identity-rejects-noncanonical-deployment-slot",
                "Deployment slot strings reject leading zeroes.",
                false,
                "leading-zero-slot",
                "invalid-projection",
            ),
            (
                "executable-identity-rejects-overflowing-deployment-slot",
                "Deployment slot strings are bounded to u64.",
                false,
                "overflow-slot",
                "invalid-projection",
            ),
            (
                "executable-identity-rejects-invalid-authority",
                "Upgrade authority variants reject incompatible fields.",
                false,
                "authority",
                "invalid-projection",
            ),
            (
                "executable-identity-rejects-invalid-payload-digest",
                "Executable payload digests require lowercase SHA-256 hexadecimal.",
                false,
                "digest",
                "invalid-projection",
            ),
            (
                "legacy-loader-rejects-program-data",
                "Immutable loader identities cannot carry upgradeable-loader fields.",
                true,
                "legacy-program-data",
                "invalid-projection",
            ),
        ] {
            let mut value = if use_legacy {
                legacy_value.clone()
            } else {
                upgradeable_value.clone()
            };
            match mutation {
                "schema" => value["schema"] = json!(PROGRAM_RELEASE_SCHEMA_V1),
                "release-field" => value["observationId"] = json!("private"),
                "program-id" => value["programId"] = json!("invalid"),
                "genesis" => value["executableIdentity"]["genesisHash"] = json!("invalid"),
                "genesis-leading-zero" => {
                    value["executableIdentity"]["genesisHash"] =
                        json!("1TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")
                }
                "identity-field" => {
                    value["executableIdentity"]["rpcUrl"] = json!("https://private.invalid")
                }
                "loader-kind" => value["executableIdentity"]["loader"]["kind"] = json!("loader-v4"),
                "loader-id" => {
                    value["executableIdentity"]["loader"]["loaderProgramId"] =
                        json!(SOLANA_BPF_LOADER_V2_PROGRAM_ID)
                }
                "program-data" => {
                    value["executableIdentity"]["loader"]["programDataAddress"] = json!("invalid")
                }
                "missing-slot" => {
                    value["executableIdentity"]["loader"]
                        .as_object_mut()
                        .unwrap()
                        .remove("deploymentSlot");
                }
                "numeric-slot" => {
                    value["executableIdentity"]["loader"]["deploymentSlot"] = json!(1)
                }
                "leading-zero-slot" => {
                    value["executableIdentity"]["loader"]["deploymentSlot"] = json!("01")
                }
                "overflow-slot" => {
                    value["executableIdentity"]["loader"]["deploymentSlot"] =
                        json!("18446744073709551616")
                }
                "authority" => {
                    value["executableIdentity"]["loader"]["upgradeAuthority"] = json!({
                        "kind": "none",
                        "address": "oreV3EG1i9BEgiAJ8b177Z2S2rMarzak4NMv1kULvWv"
                    })
                }
                "digest" => {
                    value["executableIdentity"]["loader"]["executablePayloadSha256"] =
                        json!(format!("sha256:{}", "AB".repeat(32)))
                }
                "legacy-program-data" => {
                    value["executableIdentity"]["loader"]["programDataAddress"] =
                        json!("So11111111111111111111111111111111111111112")
                }
                _ => unreachable!(),
            }
            let error = match parse_hosted_managed_program_release_v2(
                &serde_json::to_vec(&value).expect("release JSON"),
            ) {
                Ok(_) => panic!("invalid hosted release vector '{id}' was accepted"),
                Err(error) => error,
            };
            assert_eq!(error.code(), expected_code);
            self.add_failure(
                id,
                description,
                "hosted-program-release-v2",
                json!({"projection": value}),
                &error,
            );
        }
    }

    fn decoder_fixture_vectors(&mut self) {
        let normalized_idl_hash = hash_jcs::<IdlNormalized, _>(&json!({
            "fixture": "decoder-fixture-vectors"
        }))
        .expect("normalized fixture identity");
        let public_value_digest = digest_decoder_fixture_public_value_v2(&json!({
            "amount": "0",
            "state": "uninitialized"
        }))
        .expect("public fixture value digest");
        let baseline = DecoderFixtureSetV2 {
            schema: DECODER_FIXTURE_SCHEMA_V2.to_string(),
            program_id: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
            normalized_idl_hash,
            decoder_engine_id: "arete-hosted-decoder-engine/v1".to_string(),
            decoder_abi_version: "1".to_string(),
            cases: vec![
                DecoderFixtureCaseV2 {
                    id: "account-decoded".to_string(),
                    account_type: "Account".to_string(),
                    owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
                    account_data_hex: "00".to_string(),
                    expected: DecoderFixtureExpectedV2::Decoded {
                        public_value_digest,
                    },
                    expected_private_diagnostics: None,
                },
                DecoderFixtureCaseV2 {
                    id: "mint-too-short".to_string(),
                    account_type: "Mint".to_string(),
                    owner: "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA".to_string(),
                    account_data_hex: "0102".to_string(),
                    expected: DecoderFixtureExpectedV2::Error {
                        category: DecoderFixtureAccountDecodeErrorCategory::AccountTypeMismatch,
                    },
                    expected_private_diagnostics: Some(DecoderFixturePrivateDiagnosticsV2 {
                        trailing_bytes: None,
                        candidate_count: Some(1),
                    }),
                },
            ],
        };

        self.add_decoder_fixture_vector(
            "decoder-fixture-baseline",
            "Canonical private decoder fixture projection.",
            &baseline,
            Some("decoder-fixture-case-order"),
            &[],
        );

        let mut reversed = baseline.clone();
        reversed.cases.reverse();
        self.add_decoder_fixture_vector(
            "decoder-fixture-reversed-cases",
            "Case insertion order is normalized by stable case ID.",
            &reversed,
            Some("decoder-fixture-case-order"),
            &[],
        );

        let mut data_mutation = baseline.clone();
        data_mutation.cases[0].account_data_hex = "01".to_string();
        self.add_decoder_fixture_vector(
            "decoder-fixture-account-data-mutation",
            "Changing exact account bytes changes fixture identity.",
            &data_mutation,
            None,
            &["decoder-fixture-baseline"],
        );

        let mut diagnostics_mutation = baseline.clone();
        diagnostics_mutation.cases[1]
            .expected_private_diagnostics
            .as_mut()
            .unwrap()
            .candidate_count = Some(2);
        self.add_decoder_fixture_vector(
            "decoder-fixture-private-diagnostics-mutation",
            "Private diagnostic expectations participate in fixture identity.",
            &diagnostics_mutation,
            None,
            &["decoder-fixture-baseline"],
        );

        let baseline_value = serde_json::to_value(&baseline).expect("fixture serializes");
        for (id, description, mutate, code) in [
            (
                "decoder-fixture-v1-rejected",
                "Address-bearing fixture V1 fails closed.",
                "schema-v1",
                "unknown-version",
            ),
            (
                "decoder-fixture-v3-rejected",
                "Unknown future fixture schemas fail closed.",
                "schema-v3",
                "unknown-version",
            ),
            (
                "decoder-fixture-address-rejected",
                "Operational account addresses cannot enter V2 fixture bytes.",
                "address",
                "invalid-projection",
            ),
            (
                "decoder-fixture-provenance-rejected",
                "Private capture provenance cannot enter V2 fixture bytes.",
                "provenance",
                "invalid-projection",
            ),
            (
                "decoder-fixture-invalid-normalized-hash",
                "normalizedIdlHash requires the exact typed hash kind.",
                "hash",
                "invalid-projection",
            ),
            (
                "decoder-fixture-empty-engine",
                "decoderEngineId must be nonempty.",
                "engine",
                "invalid-projection",
            ),
            (
                "decoder-fixture-empty-abi",
                "decoderAbiVersion must be nonempty.",
                "abi",
                "invalid-projection",
            ),
            (
                "decoder-fixture-duplicate-case-id",
                "Case IDs are unique.",
                "duplicate",
                "invalid-projection",
            ),
            (
                "decoder-fixture-invalid-account-hex",
                "Account bytes are lowercase even-length hexadecimal.",
                "hex",
                "invalid-projection",
            ),
            (
                "decoder-fixture-invalid-error-category",
                "Error outcomes use only public AccountDecodeErrorCategory strings.",
                "category",
                "invalid-projection",
            ),
            (
                "decoder-fixture-invalid-program-id",
                "programId must be a Solana public key.",
                "program-id",
                "invalid-projection",
            ),
            (
                "decoder-fixture-leading-zero-program-id",
                "programId rejects a Base58 value with an extra leading zero byte.",
                "program-id-leading-zero",
                "invalid-projection",
            ),
            (
                "decoder-fixture-invalid-owner",
                "owner must be a Solana public key.",
                "owner",
                "invalid-projection",
            ),
            (
                "decoder-fixture-invalid-public-value-digest",
                "Decoded expectations require lowercase SHA-256 digests.",
                "digest",
                "invalid-projection",
            ),
            (
                "decoder-fixture-empty-diagnostics",
                "Private diagnostics must contain a bounded diagnostic.",
                "diagnostics-empty",
                "invalid-projection",
            ),
            (
                "decoder-fixture-zero-candidate-count",
                "candidateCount must be nonzero when present.",
                "candidate-zero",
                "invalid-projection",
            ),
            (
                "decoder-fixture-empty-cases",
                "Fixture sets require at least one case.",
                "cases-empty",
                "invalid-projection",
            ),
        ] {
            let mut value = baseline_value.clone();
            match mutate {
                "schema-v1" => value["schema"] = json!("arete.decoder-fixtures/v1"),
                "schema-v3" => value["schema"] = json!("arete.decoder-fixtures/v3"),
                "address" => {
                    value["cases"][0]["address"] =
                        json!("So11111111111111111111111111111111111111112")
                }
                "provenance" => value["rpcUrl"] = json!("https://private.invalid"),
                "hash" => {
                    value["normalizedIdlHash"] =
                        json!(hash_jcs::<IdlContent, _>(&json!({"wrong": "kind"})).unwrap())
                }
                "engine" => value["decoderEngineId"] = json!(""),
                "abi" => value["decoderAbiVersion"] = json!(""),
                "duplicate" => value["cases"][1]["id"] = value["cases"][0]["id"].clone(),
                "hex" => value["cases"][0]["accountDataHex"] = json!("ABC"),
                "category" => {
                    value["cases"][1]["expected"]["category"] = json!("account-data-too-short")
                }
                "program-id" => value["programId"] = json!("invalid"),
                "program-id-leading-zero" => {
                    value["programId"] = json!("1TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA")
                }
                "owner" => value["cases"][0]["owner"] = json!("invalid"),
                "digest" => {
                    value["cases"][0]["expected"]["publicValueDigest"] =
                        json!(format!("sha256:{}", "AB".repeat(32)))
                }
                "diagnostics-empty" => value["cases"][1]["expectedPrivateDiagnostics"] = json!({}),
                "candidate-zero" => {
                    value["cases"][1]["expectedPrivateDiagnostics"]["candidateCount"] = json!(0)
                }
                "cases-empty" => value["cases"] = json!([]),
                _ => unreachable!(),
            }
            let bytes = serde_json::to_vec(&value).unwrap();
            let error = parse_decoder_fixture_set_v2(&bytes)
                .expect_err("invalid decoder fixture must fail");
            assert_eq!(error.code(), code);
            self.add_failure(
                id,
                description,
                "decoder-fixture-set-v2",
                json!({"projection": value}),
                &error,
            );
        }
    }

    fn add_decoder_fixture_vector(
        &mut self,
        id: &str,
        description: &str,
        fixture: &DecoderFixtureSetV2,
        equivalence_group: Option<&str>,
        differs_from: &[&str],
    ) {
        let input = serde_json::to_value(fixture).expect("fixture serializes");
        let projection = fixture
            .canonical_projection()
            .expect("fixture projection validates");
        let projection_value = serde_json::to_value(&projection).unwrap();
        let payload = canonicalize_jcs(&projection).unwrap();
        let hash = fixture.hash().expect("fixture hashes");
        let hash_id = hash.to_string();
        if let Some(group) = equivalence_group {
            self.equivalence_groups
                .entry(group.to_string())
                .or_default()
                .push((id.to_string(), hash_id.clone()));
        }
        self.differs_from.extend(
            differs_from
                .iter()
                .map(|other| (id.to_string(), (*other).to_string())),
        );
        self.hash_ids_by_name.insert(id.to_string(), hash_id);
        self.decoder_fixture_vectors.push(json!({
            "id": id,
            "description": description,
            "kind": "decoder-fixture-set",
            "profile": "arete-jcs-v1",
            "equivalenceGroup": equivalence_group,
            "differsFrom": differs_from,
            "input": input,
            "expectedProjection": projection_value,
            "expected": expectation(
                HashKindName::DecoderFixtureSet,
                CanonicalizationProfile::AreteJcsV1,
                &payload,
                hash.into_any(),
            ),
        }));
    }

    fn add_release_vector<T: Serialize>(
        &mut self,
        id: &str,
        description: &str,
        projection: &T,
        metadata: Value,
        equivalence_group: Option<&str>,
        differs_from: &[&str],
    ) {
        let projection_value = serde_json::to_value(projection).expect("release serializes");
        let payload = canonicalize_jcs(&projection_value).expect("release canonicalizes");
        let hash = hash_jcs::<ProgramRelease, _>(&projection_value).expect("release hashes");
        let hash_id = hash.to_string();
        if let Some(group) = equivalence_group {
            self.equivalence_groups
                .entry(group.to_string())
                .or_default()
                .push((id.to_string(), hash_id.clone()));
        }
        self.differs_from.extend(
            differs_from
                .iter()
                .map(|other| (id.to_string(), (*other).to_string())),
        );
        self.hash_ids_by_name.insert(id.to_string(), hash_id);
        self.release_vectors.push(json!({
            "id": id,
            "description": description,
            "kind": "program-release",
            "profile": "arete-jcs-v1",
            "projectionMetadata": metadata,
            "equivalenceGroup": equivalence_group,
            "differsFrom": differs_from,
            "projection": projection_value,
            "expected": expectation(HashKindName::ProgramRelease, CanonicalizationProfile::AreteJcsV1, &payload, hash.into_any()),
        }));
    }
}

fn outcome_json(outcome: &VectorOutcome) -> Value {
    json!({
        "canonicalPayloadHex": hex::encode(&outcome.canonical_payload),
        "preimageHex": hex::encode(&outcome.preimage),
        "digestHex": hex::encode(outcome.digest),
        "hashId": outcome.hash_id.to_string(),
    })
}

fn expectation(
    kind: HashKindName,
    profile: CanonicalizationProfile,
    payload: &[u8],
    hash_id: AnyHashId,
) -> Value {
    let preimage = framed_preimage(kind, profile, payload);
    let digest: [u8; 32] = Sha256::digest(&preimage).into();
    assert_eq!(&digest, hash_id.digest());
    json!({
        "canonicalPayloadHex": hex::encode(payload),
        "preimageHex": hex::encode(preimage),
        "digestHex": hex::encode(digest),
        "hashId": hash_id.to_string(),
    })
}

fn idl_vector_json(
    id: &str,
    source: &str,
    explicit_program_id: Option<&str>,
    metadata: Value,
    document: &CanonicalIdlDocument,
    identity: &OssProgramIdentityV1,
) -> Value {
    let hashes = document.hashes();
    let source_payload = document.source_bytes();
    let content_payload = document.content_payload().expect("content payload");
    let portable_payload = document.portable_payload().expect("portable payload");
    let normalized_payload = document.normalized_payload().expect("normalized payload");
    let program_spec_payload =
        canonicalize_jcs(&identity.program_spec).expect("program spec payload");
    let release_payload = canonicalize_jcs(&identity.release).expect("release payload");

    json!({
        "id": id,
        "description": "Exact IDL source, content, portable, normalized, ProgramSpec, and OSS release projections.",
        "projectionMetadata": metadata,
        "input": {
            "encoding": "utf8",
            "data": source,
            "explicitProgramId": explicit_program_id,
        },
        "expected": {
            "programId": document.program_id(),
            "contentProjection": document.content_projection(),
            "portableProjection": document.portable_projection(),
            "normalizedSnapshot": document.normalized_snapshot(),
            "programSpec": identity.program_spec,
            "ossRelease": identity.release,
            "source": expectation(HashKindName::IdlSource, CanonicalizationProfile::RawBytesV1, source_payload, hashes.source.into_any()),
            "content": expectation(HashKindName::IdlContent, CanonicalizationProfile::AreteJcsV1, &content_payload, hashes.content.into_any()),
            "portable": expectation(HashKindName::IdlPortable, CanonicalizationProfile::AreteJcsV1, &portable_payload, hashes.portable.into_any()),
            "normalized": expectation(HashKindName::IdlNormalized, CanonicalizationProfile::AreteJcsV1, &normalized_payload, hashes.normalized.into_any()),
            "programSpecIdentity": expectation(HashKindName::ProgramSpec, CanonicalizationProfile::AreteJcsV1, &program_spec_payload, identity.program_spec_hash.into_any()),
            "ossReleaseIdentity": expectation(HashKindName::ProgramRelease, CanonicalizationProfile::AreteJcsV1, &release_payload, identity.release_hash.into_any()),
        }
    })
}

fn sample_idl(program_id: &str) -> String {
    format!(
        r#"{{
  "program": {{"publicKey": "{program_id}"}},
  "metadata": {{"name": "demo", "version": "1.0.0", "address": "{program_id}"}},
  "program_id": "{program_id}",
  "address": "{program_id}",
  "version": "1.0.0",
  "name": "demo",
  "instructions": [{{
    "name": "setValue",
    "discriminator": [1, 2, 3, 4, 5, 6, 7, 8],
    "docs": ["Set the value"],
    "accounts": [
      {{"name": "authority", "isMut": false, "isSigner": true}},
      {{"name": "vault", "isMut": true, "isSigner": false, "pda": {{"name": "vault", "seeds": [{{"kind": "const", "value": [118, 97, 117, 108, 116]}}]}}}}
    ],
    "args": [{{"name": "value", "type": "u64"}}]
  }}],
  "accounts": [],
  "types": [],
  "events": [],
  "errors": [{{"code": 6000, "name": "InvalidValue", "msg": "invalid value"}}]
}}"#
    )
}

fn sample_idl_reordered(program_id: &str) -> String {
    let value: Value = serde_json::from_str(&sample_idl(program_id)).expect("sample parses");
    let object = value.as_object().expect("sample object");
    let keys = [
        "errors",
        "events",
        "types",
        "accounts",
        "instructions",
        "name",
        "version",
        "address",
        "program_id",
        "metadata",
        "program",
    ];
    let mut fields = Vec::new();
    for key in keys {
        fields.push(format!(
            "{}:{}",
            serde_json::to_string(key).unwrap(),
            serde_json::to_string(&object[key]).unwrap()
        ));
    }
    format!("{{{}}}", fields.join(","))
}

fn sample_idl_without_program_id() -> String {
    format!("{{{}}}", minimal_idl_fields_body())
}

fn minimal_idl_fields(program_id: &str) -> String {
    if program_id.is_empty() {
        format!("{{{}}}", minimal_idl_fields_body())
    } else {
        format!(
            "{{\"address\":\"{program_id}\",{}}}",
            minimal_idl_fields_body()
        )
    }
}

fn minimal_idl_fields_body() -> &'static str {
    "\"version\":\"1.0.0\",\"name\":\"demo\",\"instructions\":[],\"accounts\":[],\"types\":[],\"events\":[],\"errors\":[]"
}
