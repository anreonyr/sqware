use crate::TaskId;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullOutcome {
    Received { len: usize, sender: TaskId },
    Discarded { len: usize, sender: TaskId },
}

impl crate::wire::FromTriple for PullOutcome {
    fn from_triple(len: usize, sender: usize, tag: usize) -> Self {
        let sender = TaskId::new(sender);
        match tag {
            0 => Self::Received { len, sender },
            1 => Self::Discarded { len, sender },
            _ => unreachable!("invalid Pull result"),
        }
    }
}
