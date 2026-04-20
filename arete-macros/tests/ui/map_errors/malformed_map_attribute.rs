use arete_macros::arete;

#[arete]
struct Broken {
    #[map(source)]
    value: u64,
}

fn main() {}
