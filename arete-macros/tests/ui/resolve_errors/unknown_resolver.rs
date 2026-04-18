use arete_macros::arete;

#[arete]
mod broken {
    #[entity(name = "Thing")]
    struct Thing {
        existing: String,
        #[resolve(from = "existing", resolver = Toke)]
        metadata: String,
    }
}

fn main() {}
