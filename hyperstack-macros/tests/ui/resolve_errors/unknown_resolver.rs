use hyperstack_macros::hyperstack;

#[hyperstack]
mod broken {
    #[entity(name = "Thing")]
    struct Thing {
        existing: String,
        #[resolve(from = "existing", resolver = Toke)]
        metadata: String,
    }
}

fn main() {}
