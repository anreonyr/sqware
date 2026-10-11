use alloc::vec::Vec;
use env::{Source, Wait};
use resource::pile::Pile;

pub struct Interests {
    pub sources: Vec<Source>,
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
    pub fn source(&mut self, source: Source) {
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
pub struct Waiting {
    pile: Pile,
    sources: Vec<Source>,
}
impl Waiting {
    pub fn new() -> Result<Self, &'static str> {
        Ok(Self {
            pile: Pile::unseal(false).map_err(|_| "shell wait allocation")?,
            sources: Vec::new(),
        })
    }
    pub fn apply(&mut self, interests: &Interests) -> Result<(), &'static str> {
        let mut at = 0;
        while at < self.sources.len() {
            if interests.sources.contains(&self.sources[at]) {
                at += 1;
            } else {
                let old = self.sources.swap_remove(at);
                let _ = self.pile.detach(old);
            }
        }
        for &source in &interests.sources {
            if !self.sources.contains(&source) {
                self.pile
                    .attach(source)
                    .map_err(|_| "shell wait registration")?;
                self.sources.push(source);
            }
        }
        Ok(())
    }
    pub fn await_(&self, within: Wait) -> Result<(), &'static str> {
        self.pile
            .await_(within)
            .map(|_| ())
            .map_err(|_| "shell wait")
    }
}
impl Drop for Waiting {
    fn drop(&mut self) {
        let _ = env::pie::release(self.pile.token(), env::ReleaseMode::Revoke);
    }
}
