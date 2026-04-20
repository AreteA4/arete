use arete_macros::arete;

#[arete]
struct Broken {
    #[resolve()]
    value: u64,
}

fn main() {}
