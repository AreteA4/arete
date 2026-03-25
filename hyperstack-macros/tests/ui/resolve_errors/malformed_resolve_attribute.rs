use hyperstack_macros::hyperstack;

#[hyperstack]
struct Broken {
    #[resolve()]
    value: u64,
}

fn main() {}
