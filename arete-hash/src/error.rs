use thiserror::Error;

#[derive(Debug, Error)]
pub enum HashError {
    #[error("invalid hash identifier: {0}")]
    InvalidHashId(&'static str),
    #[error("unknown hash protocol version '{0}'")]
    UnknownVersion(String),
    #[error("unknown hash kind '{0}'")]
    UnknownKind(String),
    #[error("expected hash kind '{expected}', got '{actual}'")]
    UnexpectedKind { expected: String, actual: String },
    #[error("unknown hash algorithm '{0}'")]
    UnknownAlgorithm(String),
    #[error("hash kind '{kind}' requires profile '{expected}', not '{actual}'")]
    ProfileMismatch {
        kind: String,
        expected: String,
        actual: String,
    },
    #[error("invalid JSON: {0}")]
    InvalidJson(String),
    #[error("duplicate JSON object key '{0}'")]
    DuplicateJsonKey(String),
    #[error("unsafe JSON integer '{0}'")]
    UnsafeJsonInteger(String),
    #[error("non-finite JSON numbers are not supported")]
    NonFiniteNumber,
    #[error("tuple labels must be unique: '{0}'")]
    DuplicateTupleLabel(String),
    #[error("invalid artifact path '{path}': {reason}")]
    InvalidArtifactPath { path: String, reason: &'static str },
    #[error("duplicate artifact path '{0}'")]
    DuplicateArtifactPath(String),
    #[error("artifact tree entries cannot be symlinks: '{0}'")]
    SymlinkArtifact(String),
    #[error("self-hash projection must be a JSON object")]
    InvalidSelfHashProjection,
    #[error("invalid {projection} projection: {reason}")]
    InvalidProjection {
        projection: &'static str,
        reason: String,
    },
    #[error("program ID is missing from the IDL and no explicit program ID was supplied")]
    MissingProgramId,
    #[error("program ID at '{location}' must be a string or null")]
    InvalidProgramIdLocation { location: &'static str },
    #[error("conflicting program IDs: {0}")]
    ConflictingProgramIds(String),
    #[error("failed to parse IDL: {0}")]
    InvalidIdl(String),
    #[error("serialization failed: {0}")]
    Serialization(String),
}

impl HashError {
    /// Stable error category used by language-neutral conformance vectors.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidHashId(_) => "invalid-hash-id",
            Self::UnknownVersion(_) => "unknown-version",
            Self::UnknownKind(_) => "unknown-kind",
            Self::UnexpectedKind { .. } => "unexpected-kind",
            Self::UnknownAlgorithm(_) => "unknown-algorithm",
            Self::ProfileMismatch { .. } => "profile-mismatch",
            Self::InvalidJson(_) => "invalid-json",
            Self::DuplicateJsonKey(_) => "duplicate-json-key",
            Self::UnsafeJsonInteger(_) => "unsafe-json-integer",
            Self::NonFiniteNumber => "non-finite-number",
            Self::DuplicateTupleLabel(_) => "duplicate-tuple-label",
            Self::InvalidArtifactPath { .. } => "invalid-artifact-path",
            Self::DuplicateArtifactPath(_) => "duplicate-artifact-path",
            Self::SymlinkArtifact(_) => "symlink-artifact",
            Self::InvalidSelfHashProjection => "invalid-self-hash-projection",
            Self::InvalidProjection { .. } => "invalid-projection",
            Self::MissingProgramId => "missing-program-id",
            Self::InvalidProgramIdLocation { .. } => "invalid-program-id-location",
            Self::ConflictingProgramIds(_) => "conflicting-program-ids",
            Self::InvalidIdl(_) => "invalid-idl",
            Self::Serialization(_) => "serialization",
        }
    }
}

impl From<serde_json::Error> for HashError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error.to_string())
    }
}
