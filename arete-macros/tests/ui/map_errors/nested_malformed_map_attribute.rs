use arete_macros::arete;

#[arete]
mod broken {
    #[derive(arete_macros::Stream)]
    struct Nested {
        #[map(source)]
        value: u64,
    }

    struct Root {
        nested: Nested,
    }
}

fn main() {}
