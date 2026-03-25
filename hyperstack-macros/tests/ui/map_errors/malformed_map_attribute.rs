use hyperstack_macros::hyperstack;

#[hyperstack]
struct Broken {
    #[map(source)]
    value: u64,
}

fn main() {}
