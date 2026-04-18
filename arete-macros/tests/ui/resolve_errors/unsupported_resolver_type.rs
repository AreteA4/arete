use arete_macros::arete;

#[arete]
mod broken {
    #[entity(name = "Thing")]
    struct Thing {
        #[resolve(from = "mint")]
        metadata: u64,
    }
}

fn main() {}
