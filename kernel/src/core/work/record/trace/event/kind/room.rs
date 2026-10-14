use serde::Serialize;

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomEvent {
    Spawn {
        tid: usize,
    },
    Switch {
        prev_tid: usize,
        next_tid: usize,
    },
    Starve {
        tid: usize,
    },
    Park {
        tid: usize,
        wake_at: usize,
    },
    Wait {
        tid: usize,
        key: usize,
    },
    Wake {
        tid: usize,
    },
    Exit {
        tid: usize,
        reason: usize,
    },
    Reap {
        tid: usize,
    },
    FaultKilled {
        tid: usize,
        cause: usize,
        stval: usize,
    },
    Doomed {
        tid: usize,
        by: usize,
    },
    Idle,
}
