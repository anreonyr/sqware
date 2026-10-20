use crate::{Bit, Decode, HoleCondition, PieToken, TaskId, TeamId, UnitTarget, Wire};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToleEventSource {
    Hole {
        token: PieToken,
        condition: HoleCondition,
    },
    Pole {
        token: PieToken,
        bit: Bit,
    },
    Nole {
        token: PieToken,
    },
    Join {
        target: UnitTarget,
    },
    Inspect {
        task: TaskId,
        token: PieToken,
    },
}

impl ToleEventSource {
    pub fn words(self) -> [usize; 3] {
        match self {
            Self::Hole { token, condition } => [1, token.get(), condition as usize],
            Self::Join {
                target: UnitTarget::Task(task),
            } => [2, task.get(), 0],
            Self::Join {
                target: UnitTarget::Team(team),
            } => [3, team.get(), 0],
            Self::Inspect { task, token } => [4, task.get(), token.get()],
            Self::Pole { token, bit } => [5, token.get(), bit.index()],
            Self::Nole { token } => [6, token.get(), 0],
        }
    }

    pub fn of(words: [usize; 3]) -> Option<Self> {
        match words {
            [1, token, condition] if token != 0 => {
                let condition = match condition {
                    0 => HoleCondition::Pull,
                    1 => HoleCondition::Push,
                    2 => HoleCondition::Empty,
                    3 => HoleCondition::Signal,
                    _ => return None,
                };
                Some(Self::Hole {
                    token: PieToken::new(token),
                    condition,
                })
            }
            [2, task, 0] => Some(Self::Join {
                target: UnitTarget::Task(TaskId::new(task)),
            }),
            [3, team, 0] => Some(Self::Join {
                target: UnitTarget::Team(TeamId::new(team)),
            }),
            [4, task, token] if token != 0 => Some(Self::Inspect {
                task: TaskId::new(task),
                token: PieToken::new(token),
            }),
            [5, token, bit] if token != 0 => Some(Self::Pole {
                token: PieToken::new(token),
                bit: Bit::of(bit)?,
            }),
            [6, token, 0] if token != 0 => Some(Self::Nole {
                token: PieToken::new(token),
            }),
            _ => None,
        }
    }
}

impl Wire for ToleEventSource {
    fn pack(&self, words: &mut [usize; 6], at: &mut usize) {
        for word in self.words() {
            word.pack(words, at);
        }
    }

    fn unpack(words: &[usize; 6], at: &mut usize) -> Result<Self, Decode> {
        let mut source = [0; 3];
        for word in &mut source {
            *word = usize::unpack(words, at)?;
        }
        Self::of(source).ok_or(Decode::Invalid)
    }
}
