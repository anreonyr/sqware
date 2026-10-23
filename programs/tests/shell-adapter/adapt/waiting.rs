use alloc::vec::Vec;
use env::{ToleEventSource, Wait};

pub struct Interests {
    pub sources: Vec<ToleEventSource>,
    pub within: Wait,
    pub runnable: bool,
}

impl Interests {
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
            within: Wait::Forever,
            runnable: false,
        }
    }

    pub fn source(&mut self, source: ToleEventSource) {
        if !self.sources.contains(&source) {
            self.sources.push(source);
        }
    }

    pub fn budget(&mut self, wait: Wait) {
        self.within = match (self.within, wait) {
            (Wait::AtMost(a), Wait::AtMost(b)) => Wait::AtMost(a.min(b)),
            (Wait::Forever, b) => b,
            (a, Wait::Forever) => a,
        };
    }
}
