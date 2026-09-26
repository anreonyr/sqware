// 结局账（ledger）— **退场的账**：哪一台、什么结局、它走时留的话。
//
// **只记不判**：内核不解释结局码的语义（见 `env::exit` 的头注），故这里没有"对/错"
// 那一格——判据在**一处调用点**（`conductor::halt` 的 testing 分支）。
//
// **只记非零**（见 [`note`]）：正常退场不占格，故"世界到底跑没跑"不能看账
// （正常跑完账是空的）——那看 `conductor::counts()`。
//
// 环 [`RING`] 条，满则覆盖最旧。**不分配**：退场路径的纪律（见 `switcher::envcall`
// 与 `messenger::doom` 的两处头注）。
//
// 写在 `messenger::quit()`——**所有退出路径的公共点**（自杀 / 他杀 / 级联 / 故障隔离），
// 与 `RoomEvent::Exit` 那一笔 trace 并排。

use env::{NOTE_MAX, Reason, TaskId};

use crate::lock::SpinLock;

/// 环的条数：够看清"哪几台塌了"，又不至于为一条正常退场付可见的成本。
const RING: usize = 8;

/// 一笔结局。
#[derive(Clone, Copy)]
pub struct Entry {
    /// 哪一台。内核只知道 tid——**名字由域自己写在 `note` 里**。
    pub task: TaskId,
    /// 什么结局（[`env::exit`] 那张码表）。
    pub reason: Reason,
    bytes: [u8; NOTE_MAX],
    len: usize,
}

impl Entry {
    const EMPTY: Self = Self {
        task: TaskId::new(0),
        reason: 0,
        bytes: [0; NOTE_MAX],
        len: 0,
    };

    /// 它走时留的那句话（没有话 = 空串；不是 UTF-8 = 一句明说它不是的替身）。
    pub fn note(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("<non-utf8 note>")
    }
}

struct Ring {
    entries: [Entry; RING],
    /// 单调递增的写入序：`written % RING` 是落点，`written - RING` 之前已覆盖。
    written: usize,
}

static LEDGER: SpinLock<Ring> = SpinLock::new(Ring {
    entries: [Entry::EMPTY; RING],
    written: 0,
});

/// 记一笔——**只记非零**。`reason == 0`（自愿/正常结束）是压倒性的常态（归档实测
/// 5498/5505），记它会把环填满、把**最早发生的那一笔**（往往正是根因）挤掉：量过一次
/// ——注入一处域内断言塌，根因那笔被后面一串正常退场冲走，dump 里只剩它引发的四笔
/// 下游塌。`note` 超 [`NOTE_MAX`] 截断。
pub fn note(task: TaskId, reason: Reason, note: &str) {
    if reason == 0 {
        return;
    }
    let raw = note.as_bytes();
    let len = raw.len().min(NOTE_MAX);
    let mut entry = Entry {
        task,
        reason,
        bytes: [0; NOTE_MAX],
        len,
    };
    entry.bytes[..len].copy_from_slice(&raw[..len]);

    let mut ring = LEDGER.lock();
    let at = ring.written % RING;
    ring.entries[at] = entry;
    ring.written += 1;
}

/// 逐笔读，**从旧到新**。
///
/// **回调里不得再调 [`note`]**：单核下会与本次遍历持有的这把锁自撞。
pub fn each(f: impl FnMut(&Entry)) {
    let mut f = f;
    let ring = LEDGER.lock();
    let start = ring.written.saturating_sub(RING);
    for i in start..ring.written {
        f(&ring.entries[i % RING]);
    }
}
