use std::fmt::Debug;

#[derive(Clone, Debug)]
pub struct OciSchema<T: Clone + Debug> {
    raw: Vec<u8>,
    item: T,
}

impl<T: Clone + Debug> OciSchema<T> {
    pub fn new(raw: Vec<u8>, item: T) -> OciSchema<T> {
        OciSchema { raw, item }
    }

    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    pub fn item(&self) -> &T {
        &self.item
    }

    pub fn into_raw(self) -> Vec<u8> {
        self.raw
    }

    pub fn into_item(self) -> T {
        self.item
    }
}
