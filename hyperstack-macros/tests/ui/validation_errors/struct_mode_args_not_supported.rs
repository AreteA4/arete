use hyperstack_macros::hyperstack;

#[hyperstack(idl = "tests/ui/fixtures/unused.json")]
struct Broken {
    value: u64,
}

fn main() {}
