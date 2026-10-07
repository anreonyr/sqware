//! 吃"发生了什么"，吐"该做什么"。
//! 事件由适配层从内核取出来交给它，它吐回的 Answer / Ring 是**数据**。等事件、取干净、
//! 收发、读表、武装、清那一格、说排空，全在适配层（`src/driver/rtc/adapt/`）——那一层只做
//! "等、取、喂、执行"，不再有自己的判定。
//! 这一层是**纯的**：不调用资源或执行原语、不出现 `View`，故可独立推理；"两位客人一个闹钟"
//! 写不出来——那一格只有两个变体（见 `slot.rs`）。

use super::frame::{self, Wire};
use super::slot::Slot;
use env::PieToken;

pub enum Answer {
    /// 「现在几点」：答一个时刻
    Time(u64),
    /// 「再过多 long 叫我」收下了：那一格占上，设备要武装到 `at`；答码是 frame::OK
    Armed {
        at: u64,
    },
    Refused {
        code: u8,
        at: u64,
    },
}

/// 一次投递：清掉设备那一格（电平源）之后，那一格到点没有
pub enum Ring {
    /// 还没到点（或者是空响）
    Quiet,
    /// 到点了：往这一枚孔推"那一声"
    Rang { back: PieToken, now: u64 },
}

/// 常驻会话核：那一格 ＋ "报了几声"那一格读数
pub struct Host {
    slot: Slot,
    rang: usize,
}

impl Host {
    /// 起一枚空会话
    pub const fn new() -> Host {
        Host {
            slot: Slot::new(),
            rang: 0,
        }
    }

    /// `back` = 客人借来的那枚孔（在本端表里的号）；`now` = 收到这一帧时设备的钟
    pub fn ask(&mut self, wire: Wire, back: PieToken, now: u64) -> Answer {
        match wire {
            Wire::Now => Answer::Time(now),
            Wire::Arm { after_ns } => {
                // **相对量在这一刻落地**：`after_ns` 是"再过多久"，故那个绝对时刻由**收帧的人**
                let at = now.saturating_add(after_ns);
                match self.slot.arm(at, back, now) {
                    Ok(()) => Answer::Armed { at },
                    // `Past` 是本面的策略、不是设备的事实（见 `slot.rs` 的 Slot::arm）。
                    Err(fail) => Answer::Refused {
                        code: frame::fail_to_code(Some(fail)),
                        at,
                    },
                }
            }
        }
    }

    /// 线上一趟投递 → 那一格到点没有（**取走就是兑现**：`Rang` 那一刻那一格已经回空）
    pub fn ring(&mut self, now: u64) -> Ring {
        match self.slot.fire(now) {
            Some(back) => Ring::Rang { back, now },
            None => Ring::Quiet,
        }
    }

    /// 那一声**推出去了**（有人收下）：读数那一格加一，返加过之后的数
    /// 推不出去时**不调用**——它数的是兑现了的那几声（与旧读数逐字同）
    pub fn heard(&mut self) -> usize {
        self.rang += 1;
        self.rang
    }
}
