use env::{Bit, MailCondition, PieKind, TaskId, TeamId};

use crate::core::memory::nest::Asid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WakeKey {
    Space { space: Asid, slot: usize },
    Hole { hole: usize, dir: MailCondition },
    Nole { id: usize },
    Pole { id: usize, bit: Bit },
    Task { id: TaskId },
    Team { id: TeamId },
    Pies { task: TaskId },
    Seal { kind: u8, id: usize },
    Inspect { task: TaskId, token: usize },
    Tole { id: usize },
    Alarm { task: TaskId },
}

impl WakeKey {
    pub(crate) fn seal(kind: PieKind, id: usize) -> Self {
        WakeKey::Seal {
            kind: kind as u8,
            id,
        }
    }

    pub(super) fn fold(self) -> u64 {
        match self {
            WakeKey::Space { space, slot } => {
                (space.get() as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ slot as u64
            }
            WakeKey::Hole { hole, dir } => {
                (hole as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ dir.wire() as u64
            }
            WakeKey::Nole { id } => (id as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9),
            WakeKey::Pole { id, bit } => {
                (id as u64).wrapping_mul(0x87C3_7B91_1142_53D5) ^ bit.index() as u64
            }
            WakeKey::Team { id } => (id.get() as u64).wrapping_mul(0x369DEA0F31A53F85),
            WakeKey::Task { id } => (id.get() as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93),
            WakeKey::Alarm { task } => (task.get() as u64).wrapping_mul(0xA24B_AED4_963E_E407),
            WakeKey::Pies { task } => (task.get() as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F),
            WakeKey::Seal { kind, id } => {
                (id as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ kind as u64
            }
            WakeKey::Inspect { task, token } => {
                (task.get() as u64).wrapping_mul(0x2545_F491_4F6C_DD1D) ^ token as u64
            }
            WakeKey::Tole { id } => (id as u64).wrapping_mul(0x1656_67B1_9E37_79F9),
        }
    }
}
