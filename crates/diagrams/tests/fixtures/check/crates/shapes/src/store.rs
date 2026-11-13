//! Where the invented tickets are kept.

pub struct Store {
    path: String,
}

impl Store {
    pub fn open(path: &str) -> Store {
        Store {
            path: path.to_owned(),
        }
    }

    pub fn tickets(&self) -> Vec<String> {
        vec![self.path.clone()]
    }
}

pub fn reset() {}
