//! Fixture: basic struct with impl block.
//! Expected extraction: Struct node + Impl node + Method nodes.

pub struct User {
    pub id: u64,
    pub name: String,
}

impl User {
    pub fn new(id: u64, name: String) -> Self {
        Self { id, name }
    }

    pub fn greet(&self) -> String {
        format!("Hello, {}!", self.name)
    }
}
