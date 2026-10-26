//! Regenerate `typescript/protocol/src/builtins.generated.ts`: the builtin
//! programs' ids, pinned ProgramSpec hashes and instruction shapes.

use arete_hash::protocol::{builtin_modules_typescript, BUILTIN_MODULES_TYPESCRIPT_PATH};

fn main() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(BUILTIN_MODULES_TYPESCRIPT_PATH);
    std::fs::write(&path, builtin_modules_typescript()).expect("write the builtin modules");
    println!("wrote {}", path.display());
}
