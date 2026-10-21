use ::resource::pile::Pile;
use alloc::vec::Vec;
use env::{HoleCondition, PieToken, ToleEventSource};

pub(crate) struct Waiting {
    pile: Pile,
    sources: Vec<ToleEventSource>,
}
impl Waiting {
    pub(crate) fn new() -> env::ToleResult<Self> {
        Ok(Self {
            pile: Pile::unseal(false)?,
            sources: Vec::new(),
        })
    }
    pub(crate) fn detach(&self, token: PieToken) {
        let _ = self.pile.detach(ToleEventSource::Hole {
            token,
            condition: HoleCondition::Pull,
        });
    }
    pub(super) fn await_(&self, wait: env::Wait) -> Result<(), ()> {
        self.pile.await_(wait).map(|_| ()).map_err(|_| ())
    }
    pub fn apply(&mut self, interests: (&[PieToken], &[PieToken]), subs: &[ToleEventSource]) -> bool {
        let (reads, writes) = interests;
        if self
            .sources
            .try_reserve(reads.len() + writes.len() + subs.len())
            .is_err()
        {
            return false;
        }
        let wanted = |source: &ToleEventSource| match source {
            ToleEventSource::Hole {
                token,
                condition: HoleCondition::Pull,
            } if reads.contains(token) => true,
            ToleEventSource::Hole {
                token,
                condition: HoleCondition::Empty,
            } if writes.contains(token) => true,
            _ => subs.contains(source),
        };
        let mut at = 0;
        while at < self.sources.len() {
            if wanted(&self.sources[at]) {
                at += 1;
            } else {
                let source = self.sources.swap_remove(at);
                let _ = self.pile.detach(source);
            }
        }
        let sources = reads
            .iter()
            .map(|&token| ToleEventSource::Hole {
                token,
                condition: HoleCondition::Pull,
            })
            .chain(writes.iter().map(|&token| ToleEventSource::Hole {
                token,
                condition: HoleCondition::Empty,
            }))
            .chain(subs.iter().copied());
        for source in sources {
            if self.sources.contains(&source) {
                continue;
            }
            if self.pile.attach(source).is_err() {
                return false;
            }
            self.sources.push(source);
        }
        true
    }
}
