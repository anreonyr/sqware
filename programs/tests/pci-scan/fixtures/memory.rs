use crate::window::FUNCTION_SIZE;

pub struct Memory {
    storage: Vec<u8>,
    start: usize,
    size: usize,
}

impl Memory {
    pub fn new(size: usize, fill: u8) -> Self {
        let storage = vec![fill; size + FUNCTION_SIZE];
        let base = storage.as_ptr() as usize;
        let start = (FUNCTION_SIZE - base % FUNCTION_SIZE) % FUNCTION_SIZE;
        Self { storage, start, size }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.storage[self.start..self.start + self.size]
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        &mut self.storage[self.start..self.start + self.size]
    }

    pub fn base(&self) -> usize {
        self.bytes().as_ptr() as usize
    }
}
