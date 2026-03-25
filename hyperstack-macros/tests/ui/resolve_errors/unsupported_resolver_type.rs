use hyperstack_macros::hyperstack;

#[hyperstack]
mod broken {
    #[entity(name = "Thing")]
    struct Thing {
        #[resolve(from = "mint")]
        metadata: u64,
    }
}

fn main() {}
