use arete_macros::arete;

#[arete]
mod broken {
    #[entity(name = "Thing")]
    struct Thing {
        #[computed(b)]
        a: u64,
        #[computed(a)]
        b: u64,
    }
}

fn main() {}
