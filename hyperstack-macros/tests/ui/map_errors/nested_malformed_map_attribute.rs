use hyperstack_macros::hyperstack;

#[hyperstack]
mod broken {
    #[derive(hyperstack_macros::Stream)]
    struct Nested {
        #[map(source)]
        value: u64,
    }

    struct Root {
        nested: Nested,
    }
}

fn main() {}
