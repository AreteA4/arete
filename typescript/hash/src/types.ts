declare const hashIdBrand: unique symbol;

export const HASH_PROTOCOL_VERSION = 1 as const;
export const HASH_PROTOCOL_LABEL = "arete-hash" as const;
export const HASH_ALGORITHM = "sha256" as const;

export const CANONICALIZATION_PROFILES = [
  "raw-bytes-v1",
  "arete-jcs-v1",
  "framed-tuple-v1",
  "artifact-tree-v1",
] as const;

export type CanonicalizationProfile =
  (typeof CANONICALIZATION_PROFILES)[number];

export type Visibility = "public" | "authenticated-owner" | "internal-only";
export type IdentityClass =
  | "exact-source"
  | "canonical-content"
  | "portable-content"
  | "normalized-content"
  | "composite"
  | "artifact-tree";

export interface IdentityMetadata<K extends string = string> {
  readonly kind: K;
  readonly profile: CanonicalizationProfile;
  readonly visibility: Visibility;
  readonly identityClass: IdentityClass;
  readonly apiField: string;
  readonly rustType: string;
  readonly typescriptType: string;
  readonly projection: string;
  readonly allowedDtoAudiences: readonly Visibility[];
  readonly databaseMappings: readonly string[];
  readonly legacyAliases: readonly string[];
}

export const IDENTITY_REGISTRY = [
  { kind: "idl-source", profile: "raw-bytes-v1", visibility: "public", identityClass: "exact-source", apiField: "sourceIdlHash", rustType: "HashId<IdlSource>", typescriptType: "IdlSourceHash", projection: "arete.idl-source/exact-bytes-v1", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: [], legacyAliases: [] },
  { kind: "idl-content", profile: "arete-jcs-v1", visibility: "public", identityClass: "canonical-content", apiField: "idlContentHash", rustType: "HashId<IdlContent>", typescriptType: "IdlContentHash", projection: "arete.idl-content/source-json-v1", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: ["idl_contents.idl_content_hash", "program_releases.idl_content_hash"], legacyAliases: ["legacy_idl_json_sha256"] },
  { kind: "idl-portable", profile: "arete-jcs-v1", visibility: "public", identityClass: "portable-content", apiField: "portableIdlHash", rustType: "HashId<IdlPortable>", typescriptType: "IdlPortableHash", projection: "arete.idl-portable/source-json-v1", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: ["idl_contents.idl_portable_hash"], legacyAliases: ["legacy_idl_json_no_program_sha256"] },
  { kind: "idl-normalized", profile: "arete-jcs-v1", visibility: "public", identityClass: "normalized-content", apiField: "normalizedIdlHash", rustType: "HashId<IdlNormalized>", typescriptType: "IdlNormalizedHash", projection: "arete.idl-normalized/v1", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: ["idl_contents.idl_normalized_hash", "decoder_bindings.normalized_idl_hash", "program_releases.normalized_idl_hash", "decoder_fixture_sets.normalized_idl_hash"], legacyAliases: ["legacy_normalized_idl_sha256"] },
  { kind: "program-spec", profile: "arete-jcs-v1", visibility: "public", identityClass: "composite", apiField: "programSpecHash", rustType: "HashId<ProgramSpec>", typescriptType: "ProgramSpecHash", projection: "arete.program-spec/v1", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: ["idl_contents.program_spec_hash", "program_spec_artifacts.program_spec_hash", "program_releases.program_spec_hash"], legacyAliases: [] },
  { kind: "ast-portable", profile: "arete-jcs-v1", visibility: "public", identityClass: "portable-content", apiField: "portableAstHash", rustType: "HashId<AstPortable>", typescriptType: "AstPortableHash", projection: "arete.ast-portable/self-hash-v1", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: ["ast_contents.ast_portable_hash", "builds.ast_portable_hash", "deployments.current_ast_portable_hash"], legacyAliases: ["legacy_portable_ast_sha256"] },
  { kind: "runtime-artifact", profile: "arete-jcs-v1", visibility: "internal-only", identityClass: "composite", apiField: "runtimeArtifactHash", rustType: "HashId<RuntimeArtifact>", typescriptType: "RuntimeArtifactHash", projection: "arete.runtime-artifact/v1", allowedDtoAudiences: ["internal-only"], databaseMappings: ["runtime_artifacts.runtime_artifact_hash", "builds.runtime_artifact_hash"], legacyAliases: ["legacy_platform_ast_sha256"] },
  { kind: "artifact-file", profile: "raw-bytes-v1", visibility: "public", identityClass: "canonical-content", apiField: "artifactFileHash", rustType: "HashId<ArtifactFile>", typescriptType: "ArtifactFileHash", projection: "arete.artifact-file/exact-bytes-v1", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: [], legacyAliases: [] },
  { kind: "decoder-content", profile: "raw-bytes-v1", visibility: "internal-only", identityClass: "canonical-content", apiField: "decoderContentHash", rustType: "HashId<DecoderContent>", typescriptType: "DecoderContentHash", projection: "arete.decoder-content/exact-bytes-v1", allowedDtoAudiences: ["internal-only"], databaseMappings: ["decoder_contents.content_hash", "decoder_executions.decoder_content_hash"], legacyAliases: ["legacy_decoder_content_sha256"] },
  { kind: "sdk-definition", profile: "arete-jcs-v1", visibility: "public", identityClass: "composite", apiField: "sdkDefinitionHash", rustType: "HashId<SdkDefinition>", typescriptType: "SdkDefinitionHash", projection: "arete.sdk-definition/v1", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: [], legacyAliases: [] },
  { kind: "sdk-extension", profile: "arete-jcs-v1", visibility: "public", identityClass: "composite", apiField: "sdkExtensionHash", rustType: "HashId<SdkExtension>", typescriptType: "SdkExtensionHash", projection: "arete.sdk-extension/v1", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: ["sdk_extension_contents.sdk_extension_hash"], legacyAliases: ["legacy_sdk_extension_sha256"] },
  { kind: "sdk-output-tree", profile: "artifact-tree-v1", visibility: "public", identityClass: "artifact-tree", apiField: "sdkOutputTreeHash", rustType: "HashId<SdkOutputTree>", typescriptType: "SdkOutputTreeHash", projection: "arete.sdk-output-tree/artifact-tree-v1", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: ["sdk_extension_contents.sdk_output_tree_hash"], legacyAliases: [] },
  { kind: "compiler", profile: "framed-tuple-v1", visibility: "public", identityClass: "composite", apiField: "compilerHash", rustType: "HashId<Compiler>", typescriptType: "CompilerHash", projection: "arete.compiler/v1", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: [], legacyAliases: [] },
  { kind: "program-release", profile: "arete-jcs-v1", visibility: "public", identityClass: "composite", apiField: "programReleaseHash", rustType: "HashId<ProgramRelease>", typescriptType: "ProgramReleaseHash", projection: "arete.program-release/v1", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: ["program_releases.release_hash"], legacyAliases: [] },
  { kind: "live-spec", profile: "arete-jcs-v1", visibility: "public", identityClass: "composite", apiField: "liveSpecHash", rustType: "HashId<LiveSpec>", typescriptType: "LiveSpecHash", projection: "arete.artifact-envelope/live-spec-v1", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: ["live_spec_artifacts.live_spec_hash"], legacyAliases: [] },
  { kind: "stack-manifest", profile: "arete-jcs-v1", visibility: "public", identityClass: "composite", apiField: "stackManifestHash", rustType: "HashId<StackManifest>", typescriptType: "StackManifestHash", projection: "arete.artifact-envelope/stack-manifest-v1", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: ["stack_manifest_artifacts.stack_manifest_hash"], legacyAliases: [] },
  { kind: "deployment-release", profile: "arete-jcs-v1", visibility: "authenticated-owner", identityClass: "composite", apiField: "deploymentReleaseHash", rustType: "HashId<DeploymentRelease>", typescriptType: "DeploymentReleaseHash", projection: "arete.deployment-release/v1", allowedDtoAudiences: ["authenticated-owner", "internal-only"], databaseMappings: ["deployment_releases.deployment_release_hash", "builds.deployment_release_hash", "deployments.deployment_release_hash"], legacyAliases: [] },
  { kind: "decoder-fixture-set", profile: "arete-jcs-v1", visibility: "internal-only", identityClass: "composite", apiField: "decoderFixtureSetHash", rustType: "HashId<DecoderFixtureSet>", typescriptType: "DecoderFixtureSetHash", projection: "arete.decoder-fixtures/v2", allowedDtoAudiences: ["internal-only"], databaseMappings: ["decoder_fixture_sets.fixture_set_hash"], legacyAliases: [] },
  { kind: "knowledge-document", profile: "arete-jcs-v1", visibility: "public", identityClass: "canonical-content", apiField: "documentHash", rustType: "HashId<KnowledgeDocument>", typescriptType: "KnowledgeDocumentHash", projection: "arete.knowledge-document/v1", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: ["knowledge_document_artifacts.document_hash", "knowledge_snapshot_documents.document_hash", "catalog_bundle_documents.document_hash"], legacyAliases: [] },
  { kind: "knowledge-snapshot", profile: "arete-jcs-v1", visibility: "public", identityClass: "composite", apiField: "knowledgeSnapshotHash", rustType: "HashId<KnowledgeSnapshot>", typescriptType: "KnowledgeSnapshotHash", projection: "arete.knowledge-snapshot/v1", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: ["knowledge_snapshot_artifacts.snapshot_hash", "catalog_publication_sets.knowledge_snapshot_hash"], legacyAliases: [] },
  { kind: "extension-surface", profile: "arete-jcs-v1", visibility: "public", identityClass: "composite", apiField: "surfaceHash", rustType: "HashId<ExtensionSurface>", typescriptType: "ExtensionSurfaceHash", projection: "arete.extension-surface/v2", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: ["extension_surface_artifacts_v2.surface_hash", "sdk_install_target_artifacts.surface_hash", "catalog_bundle_surfaces.surface_hash"], legacyAliases: [] },
  { kind: "sdk-install-target", profile: "arete-jcs-v1", visibility: "public", identityClass: "composite", apiField: "sdkInstallTargetHash", rustType: "HashId<SdkInstallTarget>", typescriptType: "SdkInstallTargetHash", projection: "arete.sdk-install-target/v1", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: ["sdk_install_target_artifacts.sdk_install_target_hash", "registry_package_sdk_targets.sdk_install_target_hash", "catalog_bundle_sdk_targets.sdk_install_target_hash"], legacyAliases: [] },
  { kind: "catalog-bundle", profile: "arete-jcs-v1", visibility: "public", identityClass: "composite", apiField: "bundleHash", rustType: "HashId<CatalogBundle>", typescriptType: "CatalogBundleHash", projection: "arete.catalog-bundle/v1", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: ["catalog_bundles.bundle_hash", "catalog_publication_set_entries.bundle_hash"], legacyAliases: [] },
  { kind: "catalog-publication-set", profile: "arete-jcs-v1", visibility: "public", identityClass: "composite", apiField: "setHash", rustType: "HashId<CatalogPublicationSet>", typescriptType: "CatalogPublicationSetHash", projection: "arete.catalog-publication-set/v1", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: ["catalog_publication_sets.set_hash", "catalog_active_sets.set_hash", "catalog_publication_events.set_hash"], legacyAliases: [] },
] as const satisfies readonly IdentityMetadata<string>[];

export interface NonHashIdentityMetadata {
  readonly apiField: string;
  readonly rustType: string;
  readonly typescriptType: string;
  readonly projection: string;
  readonly visibility: Visibility;
  readonly allowedDtoAudiences: readonly Visibility[];
  readonly databaseMappings: readonly string[];
  readonly legacyAliases: readonly string[];
}

export const NON_HASH_IDENTITY_REGISTRY = [
  { apiField: "programReadBindingId", rustType: "ProgramReadBindingId", typescriptType: "ProgramReadBindingId", projection: "arete.program-read-binding/v1", visibility: "public", allowedDtoAudiences: ["public", "authenticated-owner", "internal-only"], databaseMappings: ["program_read_bindings.id", "program_read_routes.program_read_binding_id", "program_read_usage_events.program_read_binding_id"], legacyAliases: [] },
  { apiField: "decoderBindingId", rustType: "internal::DecoderBindingId", typescriptType: "DecoderBindingId", projection: "arete.decoder-binding/v1", visibility: "internal-only", allowedDtoAudiences: ["internal-only"], databaseMappings: ["decoder_bindings.id", "program_releases.decoder_binding_id"], legacyAliases: [] },
  { apiField: "decoderEngineId", rustType: "internal::DecoderEngineId", typescriptType: "DecoderEngineId", projection: "arete.decoder-engine/v1", visibility: "internal-only", allowedDtoAudiences: ["internal-only"], databaseMappings: ["decoder_executions.decoder_engine_id", "program_releases.decoder_engine_id", "decoder_fixture_sets.decoder_engine_id"], legacyAliases: [] },
] as const satisfies readonly NonHashIdentityMetadata[];

type IdentityRegistryEntry = (typeof IDENTITY_REGISTRY)[number];
export type HashKind = IdentityRegistryEntry["kind"];
export type HashKindForVisibility<V extends Visibility> = Extract<
  IdentityRegistryEntry,
  { readonly visibility: V }
>["kind"];
export type HashKindForProfile<P extends CanonicalizationProfile> = Extract<
  IdentityRegistryEntry,
  { readonly profile: P }
>["kind"];
export type PublicHashKind = HashKindForVisibility<"public">;
export type AuthenticatedOwnerHashKind = HashKindForVisibility<"authenticated-owner">;
export type OwnerHashKind = AuthenticatedOwnerHashKind;
export type InternalHashKind = HashKindForVisibility<"internal-only">;
export type RawBytesHashKind = HashKindForProfile<"raw-bytes-v1">;
export type JcsHashKind = HashKindForProfile<"arete-jcs-v1">;
export type TupleHashKind = HashKindForProfile<"framed-tuple-v1">;
export type ArtifactTreeHashKind = HashKindForProfile<"artifact-tree-v1">;

export type HashId<K extends HashKind = HashKind> =
  `arete:h1:${K}:sha256:${string}` & { readonly [hashIdBrand]: K };

export type PublicHashId<K extends PublicHashKind = PublicHashKind> = HashId<K>;
export type IdlSourceHash = HashId<"idl-source">;
export type IdlContentHash = HashId<"idl-content">;
export type IdlPortableHash = HashId<"idl-portable">;
export type IdlNormalizedHash = HashId<"idl-normalized">;
export type ProgramSpecHash = HashId<"program-spec">;
export type AstPortableHash = HashId<"ast-portable">;
export type RuntimeArtifactHash = HashId<"runtime-artifact">;
export type ArtifactFileHash = HashId<"artifact-file">;
export type DecoderContentHash = HashId<"decoder-content">;
export type SdkDefinitionHash = HashId<"sdk-definition">;
export type SdkExtensionHash = HashId<"sdk-extension">;
export type SdkOutputTreeHash = HashId<"sdk-output-tree">;
export type CompilerHash = HashId<"compiler">;
export type ProgramReleaseHash = HashId<"program-release">;
export type LiveSpecHash = HashId<"live-spec">;
export type StackManifestHash = HashId<"stack-manifest">;
export type DeploymentReleaseHash = HashId<"deployment-release">;
export type DecoderFixtureSetHash = HashId<"decoder-fixture-set">;
export type KnowledgeDocumentHash = HashId<"knowledge-document">;
export type KnowledgeSnapshotHash = HashId<"knowledge-snapshot">;
export type ExtensionSurfaceHash = HashId<"extension-surface">;
export type SdkInstallTargetHash = HashId<"sdk-install-target">;
export type CatalogBundleHash = HashId<"catalog-bundle">;
export type CatalogPublicationSetHash = HashId<"catalog-publication-set">;

export interface ParsedHashId<K extends HashKind = HashKind> {
  readonly id: HashId<K>;
  readonly kind: K;
  readonly digest: Uint8Array;
  readonly digestHex: string;
}

export type JsonPrimitive = null | boolean | number | string;
export type JsonValue = JsonPrimitive | JsonValue[] | { [key: string]: JsonValue };

export interface TupleField {
  readonly label: string;
  readonly value: Uint8Array;
}

export interface ArtifactTreeFileEntry {
  readonly path: string;
  readonly bytes: Uint8Array;
  readonly type?: "file";
}

export interface ArtifactTreeSymlinkEntry {
  readonly path: string;
  readonly bytes?: Uint8Array;
  readonly type: "symlink";
}

export type ArtifactTreeEntry = ArtifactTreeFileEntry | ArtifactTreeSymlinkEntry;

export function identityMetadata<K extends HashKind>(kind: K): IdentityMetadata<K> {
  const metadata = IDENTITY_REGISTRY.find((item) => item.kind === kind);
  if (!metadata) throw new Error(`unknown hash kind '${kind}'`);
  return metadata as unknown as IdentityMetadata<K>;
}
