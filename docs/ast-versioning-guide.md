# AST Versioning Guide

This guide explains how to add breaking changes to the AST and bump the version while maintaining backward compatibility.

## Overview

HyperStack uses **semantic versioning** for AST schemas (major.minor.patch):

- **Major (X.0.0)**: Breaking structural changes (renamed fields, removed fields)
- **Minor (0.X.0)**: New optional fields, additions that don't break old code
- **Patch (0.0.X)**: Bug fixes, documentation changes

## Quick Reference

| Change Type | Version Bump | Migration Required? |
|------------|--------------|-------------------|
| Add new optional field | Minor (0.0.1 → 1.1.0) | No |
| Rename field | Major (0.0.1 → 2.0.0) | Yes |
| Remove field | Major (0.0.1 → 2.0.0) | Yes |
| Change field type | Major (0.0.1 → 2.0.0) | Yes |
| Restructure enum | Major (0.0.1 → 2.0.0) | Yes |

## Step-by-Step: Adding a Breaking Change

### Step 1: Define the New AST Version

**⚠️ CRITICAL: You must update the version constant in BOTH crates.**

The AST types are duplicated between `hyperstack-macros` (for compile-time code generation) and `interpreter` (for runtime) due to circular dependency constraints (proc-macro crates cannot depend on their output crates). Both crates must have the same `CURRENT_AST_VERSION` constant.

**`hyperstack-macros/src/ast/types.rs`**
```rust
// Change this
pub const CURRENT_AST_VERSION: &str = "0.0.1";

// To this (for a minor bump)
pub const CURRENT_AST_VERSION: &str = "1.1.0";

// Or this (for a major bump)
pub const CURRENT_AST_VERSION: &str = "2.0.0";
```

**`interpreter/src/ast.rs`**
```rust
// Mirror the EXACT same change here
pub const CURRENT_AST_VERSION: &str = "2.0.0";
```

**Why two places?** The AST types exist in both crates:
- `hyperstack-macros`: Used at compile time when processing `#[hyperstack]` attributes
- `interpreter`: Used at runtime and for CLI tools (SDK generation, etc.)

**Don't worry about forgetting:** There's a test (`test_ast_version_sync_*`) in both crates that will fail if the constants get out of sync. You'll see an error like:
```
AST version mismatch! hyperstack-macros has '0.0.1', interpreter has '2.0.0'.
Both crates must have the same CURRENT_AST_VERSION.
Update both files when bumping the version.
```

### Step 2: Create the New AST Structure

Define the new version of your types. You have two options:

#### Option A: In-Place Changes (Recommended for Minor Bumps)

For minor bumps (adding optional fields), just modify the existing struct:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableStreamSpec {
    #[serde(default = "default_ast_version")]
    pub ast_version: String,
    pub state_name: String,
    // ... existing fields ...
    
    // NEW in v1.1.0
    #[serde(default)]
    pub new_field: Option<String>,
}
```

#### Option B: Separate Types (Required for Major Bumps)

For major changes, create new struct definitions:

**`hyperstack-macros/src/ast/types.rs`**
```rust
// Keep old version for migration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableStreamSpecV1 {
    pub state_name: String,
    pub old_field: String,  // This will be removed in v2
    // ... other v1 fields ...
}

// New v2 structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableStreamSpecV2 {
    #[serde(default = "default_ast_version")]
    pub ast_version: String,
    pub state_name: String,
    pub new_field: String,  // Renamed from old_field
    // ... other v2 fields ...
}

// Keep the main type as latest
pub type SerializableStreamSpec = SerializableStreamSpecV2;
```

### Step 3: Add Migration Logic

Update the versioned loader in **both** crates:

**`hyperstack-macros/src/ast/versioned.rs`**

```rust
pub fn load_stream_spec(json: &str) -> Result<SerializableStreamSpec, VersionedLoadError> {
    let raw: Value = serde_json::from_str(json)
        .map_err(|e| VersionedLoadError::InvalidJson(e.to_string()))?;
    
    let version = raw
        .get("ast_version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.0.1");
    
    match version {
        "0.0.1" => {
            serde_json::from_value::<SerializableStreamSpec>(raw)
                .map_err(|e| VersionedLoadError::InvalidStructure(e.to_string()))
        }
        "2.0.0" => {
            // v2 is current, deserialize directly
            serde_json::from_value::<SerializableStreamSpec>(raw)
                .map_err(|e| VersionedLoadError::InvalidStructure(e.to_string()))
        }
        "0.0.1" => {
            // OLD: Load v1 and migrate to v2
            let v1: SerializableStreamSpecV1 = serde_json::from_value(raw)
                .map_err(|e| VersionedLoadError::InvalidStructure(e.to_string()))?;
            Ok(migrate_stream_v1_to_v2(v1))
        }
        _ => Err(VersionedLoadError::UnsupportedVersion(version.to_string())),
    }
}

// Migration function
fn migrate_stream_v1_to_v2(v1: SerializableStreamSpecV1) -> SerializableStreamSpec {
    SerializableStreamSpec {
        ast_version: CURRENT_AST_VERSION.to_string(),
        state_name: v1.state_name,
        new_field: transform_old_field(v1.old_field),  // Transform data
        // ... migrate other fields ...
    }
}
```

Do the same for `interpreter/src/versioned.rs`.

### Step 4: Update Versioned Enums (Optional)

If you're using the `VersionedStreamSpec` enum for explicit version handling:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "ast_version")]
pub enum VersionedStreamSpec {
    #[serde(rename = "0.0.1")]
    V1(SerializableStreamSpecV1),
    #[serde(rename = "2.0.0")]
    V2(SerializableStreamSpec),  // Current version
}

impl VersionedStreamSpec {
    pub fn into_latest(self) -> SerializableStreamSpec {
        match self {
            VersionedStreamSpec::V1(v1) => migrate_stream_v1_to_v2(v1),
            VersionedStreamSpec::V2(v2) => v2,
        }
    }
}
```

### Step 5: Update All Constructors

Find all places that construct the spec and add the new `ast_version` field:

```rust
// Old
let spec = SerializableStreamSpec {
    state_name: "MyEntity".to_string(),
    old_field: "value".to_string(),
    // ...
};

// New
let spec = SerializableStreamSpec {
    ast_version: CURRENT_AST_VERSION.to_string(),  // ADD THIS
    state_name: "MyEntity".to_string(),
    new_field: transform_value("value"),  // Updated field
    // ...
};
```

Common locations to check:
- `hyperstack-macros/src/stream_spec/ast_writer.rs`
- `hyperstack-macros/src/stream_spec/module.rs`
- `hyperstack-macros/src/stream_spec/idl_spec.rs`
- `interpreter/src/ast.rs` (to_serializable methods)
- `interpreter/src/typescript.rs` (test specs)

### Step 6: Write Tests

Add tests to verify migration works correctly:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migrate_v1_to_v2() {
        let v1_json = r#"
        {
            "ast_version": "0.0.1",
            "state_name": "TestEntity",
            "old_field": "old_value"
        }
        "#;

        let spec = load_stream_spec(v1_json).unwrap();
        assert_eq!(spec.ast_version, "2.0.0");
        assert_eq!(spec.state_name, "TestEntity");
        assert_eq!(spec.new_field, "transformed_value");
    }

    #[test]
    fn test_load_v2_directly() {
        let v2_json = r#"
        {
            "ast_version": "2.0.0",
            "state_name": "TestEntity",
            "new_field": "new_value"
        }
        "#;

        let spec = load_stream_spec(v2_json).unwrap();
        assert_eq!(spec.ast_version, "2.0.0");
        assert_eq!(spec.new_field, "new_value");
    }
}
```

### Step 7: Deprecation Window (Optional)

If you want to deprecate old versions after a certain period:

```rust
pub fn load_stream_spec(json: &str) -> Result<SerializableStreamSpec, VersionedLoadError> {
    // ... version detection ...
    
    match version {
        "0.0.1" => {
            // Log deprecation warning
            eprintln!("WARNING: Loading deprecated AST v0.0.1. Please upgrade your AST files.");
            
            let v1: SerializableStreamSpecV1 = serde_json::from_value(raw)
                .map_err(|e| VersionedLoadError::InvalidStructure(e.to_string()))?;
            Ok(migrate_stream_v1_to_v2(v1))
        }
        // ...
    }
}
```

After your deprecation period, you can remove support:

```rust
"0.0.1" => {
    Err(VersionedLoadError::UnsupportedVersion(
        "0.0.1 (deprecated, please upgrade your AST files)".to_string()
    ))
}
```

## Complete Example: Field Rename

Let's say we want to rename `old_name` to `new_name` in v2.0.0:

### 1. Define v1 structure (in versioned.rs)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableStreamSpecV1 {
    pub state_name: String,
    pub old_name: String,
}
```

### 2. Update main type

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializableStreamSpec {
    #[serde(default = "default_ast_version")]
    pub ast_version: String,
    pub state_name: String,
    pub new_name: String,  // Renamed from old_name
}
```

### 3. Add migration

```rust
fn migrate_stream_v1_to_v2(v1: SerializableStreamSpecV1) -> SerializableStreamSpec {
    SerializableStreamSpec {
        ast_version: "2.0.0".to_string(),
        state_name: v1.state_name,
        new_name: v1.old_name,  // Direct mapping
    }
}

pub fn load_stream_spec(json: &str) -> Result<SerializableStreamSpec, VersionedLoadError> {
    let raw: Value = serde_json::from_str(json)?;
    let version = raw.get("ast_version").and_then(|v| v.as_str()).unwrap_or("0.0.1");
    
    match version {
        "0.0.1" => {
            let v1: SerializableStreamSpecV1 = serde_json::from_value(raw)?;
            Ok(migrate_stream_v1_to_v2(v1))
        }
        "2.0.0" => {
            serde_json::from_value::<SerializableStreamSpec>(raw)
                .map_err(|e| VersionedLoadError::InvalidStructure(e.to_string()))
        }
        _ => Err(VersionedLoadError::UnsupportedVersion(version.to_string())),
    }
}
```

### 4. Test both directions

```rust
#[test]
fn test_v1_migration() {
    let json = r#"{"ast_version":"0.0.1","state_name":"Test","old_name":"Value"}"#;
    let spec = load_stream_spec(json).unwrap();
    assert_eq!(spec.new_name, "Value");
    assert_eq!(spec.ast_version, "2.0.0");
}

#[test]
fn test_v2_native() {
    let json = r#"{"ast_version":"2.0.0","state_name":"Test","new_name":"Value"}"#;
    let spec = load_stream_spec(json).unwrap();
    assert_eq!(spec.new_name, "Value");
}
```

## Best Practices

1. **Always bump both crates** - Keep versions in sync between `hyperstack-macros` and `interpreter`

2. **Keep old versions for 6+ months** - Give users time to upgrade their pipelines

3. **Log migration warnings** - Let users know when their AST is being migrated

4. **Test edge cases** - Missing fields, null values, malformed data

5. **Document in CHANGELOG** - Note AST version changes in release notes

6. **Use serde defaults** - For minor bumps, use `#[serde(default)]` to avoid breaking old ASTs

7. **Validate after migration** - Ensure migrated ASTs pass validation

## Migration Checklist

Before releasing a new AST version:

- [ ] Updated `CURRENT_AST_VERSION` in both type files
- [ ] Added migration logic in both versioned.rs files
- [ ] Updated all spec constructors
- [ ] Added tests for migration
- [ ] Tested loading old ASTs
- [ ] Tested loading new ASTs
- [ ] Updated documentation
- [ ] Added CHANGELOG entry
- [ ] Considered deprecation timeline

## FAQ

**Q: Can I skip versions? (e.g., 0.0.1 → 3.0.0)**

A: Yes, but you must support all intermediate versions. If someone has v0.0.1 and you skip to v3.0.0, you need:
- 0.0.1 → 2.0.0 migration
- 2.0.0 → 3.0.0 migration

**Q: What if I need to rollback?**

A: AST versions are additive. Keep old migration code and tests. Users with new ASTs can't go back, but old ASTs continue working.

**Q: How do I deprecate a version?**

A: After your deprecation window, change the migration to return an error:

```rust
"0.0.1" => Err(VersionedLoadError::UnsupportedVersion(
    "0.0.1 deprecated, run: hyperstack migrate-ast".to_string()
))
```

**Q: Can I automate AST upgrades?**

A: Yes! Create a CLI command that:
1. Loads old AST
2. Migrates to latest
3. Writes back to file

```rust
pub fn upgrade_ast_file(path: &Path) -> Result<()> {
    let json = fs::read_to_string(path)?;
    let spec = load_stream_spec(&json)?;  // Auto-migrates
    let upgraded = serde_json::to_string_pretty(&spec)?;
    fs::write(path, upgraded)?;
    Ok(())
}
```