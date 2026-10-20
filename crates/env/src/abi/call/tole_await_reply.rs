use super::{ToleEventFail, ToleEventSource};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToleAwaitReply {
    Pending,
    Source {
        source: ToleEventSource,
        fail: Option<ToleEventFail>,
    },
}

impl ToleAwaitReply {
    pub fn is_none(self) -> bool {
        self == Self::Pending
    }

    pub fn words(self) -> [usize; 3] {
        match self {
            Self::Pending => [0; 3],
            Self::Source { source, fail } => {
                let mut words = source.words();
                words[0] |= fail.map_or(0, |e| (-crate::FailCode::code(e)) as usize) << 8;
                words
            }
        }
    }

    pub fn of(mut words: [usize; 3]) -> Option<Self> {
        if words == [0; 3] {
            return Some(Self::Pending);
        }
        let fail = match words[0] >> 8 {
            0 => None,
            1 => Some(ToleEventFail::Denied),
            2 => Some(ToleEventFail::Dead),
            3 => Some(ToleEventFail::Busy),
            4 => Some(ToleEventFail::OoM),
            5 => Some(ToleEventFail::HandedOver),
            6 => Some(ToleEventFail::Gone),
            _ => return None,
        };
        words[0] &= 255;
        Some(Self::Source {
            source: ToleEventSource::of(words)?,
            fail,
        })
    }
}

impl crate::wire::FromTriple for ToleAwaitReply {
    fn from_triple(a: usize, b: usize, c: usize) -> Self {
        Self::of([a, b, c]).expect("invalid Tole Await reply")
    }
}
