//! echo::adapt::echo — **回显那一圈（壳）**：从控制台那枚孔读一批，按行写回去；读到收场词退场。
//!
//! **一次读就是一批**（服务排空多少给多少，也可能是半行）：**阻塞读**，没有字节就在核里睡——
//! 不轮询、不空转（从前的 `IDLE_MS` 那一拍随调试面读入一起不用了；那条"空转的红线"见
//! `kernel/src/console.rs` 与 `kernel/src/runtime/switcher/envcall/debug.rs`）。
//! `Err` = 那枚孔封了（持设备的域没了）或装不下（见 [`BUF_MAX`]）⇒ 收场。
//!
//! **判定一条也不在这里**：行尾是哪两个字节、收场词是哪一个、非 UTF-8 怎么折，全在
//! [`crate::core::line::Line`]——本文件只做"读一批、喂给它、把攒成的行写出去"。

use crate::core::line::Line;
use env::DBCN_MAX;
use protocol::debug;
use runtime::env::mail::HolePie;

/// 一次从控制台读多少字节。
///
/// **必须 ≥ 对面一次排空的上界**（今天 `DRAIN_MAX` = 64）：孔那一格**装不下就答 `Denied`
/// 且一个字节都不动**（内核 `pull` 的口径，不是截断）⇒ 缓冲小了不是丢一行，是**读不动**。
/// 取 [`DBCN_MAX`]（256）留四倍余量。
const BUF_MAX: usize = DBCN_MAX;

/// 回显到收场：读到收场词、或那枚孔读不动了。
///
/// 两条路都是**正常收场**（那台设备没了 ⇒ 回显这件事已经没有可继续的状态），故没有失败那一格
/// ——`main` 那一格因此只报"一次往返都做不成"（找不到控制台）。
pub fn run(console: &HolePie) {
    let mut buf = [0u8; BUF_MAX];
    let mut line = Line::new();

    loop {
        let Ok(n) = console.pull(&mut buf) else { break };
        let Some(chunk) = buf.get(..n) else { break };

        for &b in chunk {
            if !Line::ends(b) {
                line.put(b);
                continue;
            }
            if line.exit() {
                return;
            }
            // 这一行攒完了：写出去、从头再攒。
            debug!("{}", line.word());
            line.clear();
        }
    }
}
