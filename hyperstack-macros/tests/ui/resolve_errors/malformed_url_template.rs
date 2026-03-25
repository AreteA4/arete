use hyperstack_macros::hyperstack;

#[hyperstack]
mod broken {
    #[entity(name = "Thing")]
    struct Thing {
        #[resolve(url = "https://example.com/{mint", extract = "name")]
        metadata: String,
    }
}

fn main() {}
