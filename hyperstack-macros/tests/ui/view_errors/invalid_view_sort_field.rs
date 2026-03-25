use hyperstack_macros::hyperstack;

#[hyperstack]
mod broken {
    #[entity(name = "Thing")]
    #[view(name = "latest", sort_by = "ghost.value")]
    struct Thing {
        base: u64,
    }
}

fn main() {}
