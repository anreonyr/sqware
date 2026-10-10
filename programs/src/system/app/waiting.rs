use ::resource::pile::Pile;
use alloc::vec::Vec;
use env::{MailCondition, PieToken, Source};

pub(crate) struct Waiting {
    pile: Pile,
    sources: Vec<Source>,
}
impl Waiting {
    pub(crate) fn new() -> Result<Self, ()> {
        Ok(Self { pile: Pile::unseal(false).map_err(|_| ())?, sources: Vec::new() })
    }
    pub(crate) fn detach(&self, token: PieToken) {
        let _ = self.pile.detach(Source::Mail { pie: token, condition: MailCondition::Pull });
    }
    pub(super) fn await_(&self, wait: env::Wait) -> Result<(), ()> {
        self.pile.await_(wait).map(|_| ()).map_err(|_| ())
    }
    pub fn apply(&mut self, interests: (&[PieToken], &[PieToken]), subs: &[Source]) -> bool {
        let (reads, writes) = interests;
        if self.sources.try_reserve(reads.len() + writes.len() + subs.len()).is_err() { return false; }
        let wanted = |source: &Source| match source {
            Source::Mail { pie, condition: MailCondition::Pull } if reads.contains(pie) => true,
            Source::Mail { pie, condition: MailCondition::Empty } if writes.contains(pie) => true,
            _ => subs.contains(source),
        };
        let mut at = 0;
        while at < self.sources.len() {
            if wanted(&self.sources[at]) { at += 1; } else {
                let source = self.sources.swap_remove(at);
                let _ = self.pile.detach(source);
            }
        }
        let sources = reads.iter().map(|&pie| Source::Mail { pie, condition: MailCondition::Pull })
            .chain(writes.iter().map(|&pie| Source::Mail { pie, condition: MailCondition::Empty }))
            .chain(subs.iter().copied());
        for source in sources {
            if self.sources.contains(&source) { continue; }
            if self.pile.attach(source).is_err() { return false; }
            self.sources.push(source);
        }
        true
    }
}
