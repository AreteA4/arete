use arete_macros::arete;

#[arete(idl = "tests/ui/fixtures/unused.json")]
struct Broken {
    value: u64,
}

fn main() {}
