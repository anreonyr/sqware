//! echo::adapt::echo — **回显那一圈（壳）**：从控制台的**读口**读一批，逐字节喂给 [`Line`]，
//! 攒成一行就往**写口**推一条字；收场词到、或任一枚孔读/推不动了，收场。
//!
//! **一次读就是一批**（对面排空多少给多少，也可能是半行）：`pull` **阻塞**，没有字节就在核里
//! 睡——不轮询、不空转（那条红线在 `kernel/src/console.rs`）。
//!
//! **判定一条也不在这里**：行尾、收场词、非 UTF-8 折法、CRLF 算几次断行，全在 [`Line::feed`]。
//! 本文件只做"读一批 → 喂进去 → 把交出来的每一行推给控制台"；连"攒完清零"都不必记得（账在
//! `feed` 里清），故这里没有半条规则。**换行本域补**（终端那一侧的约定）：一条消息 = 一条完整的字。
//!
//! **收场三个来路**：读到收场词、读口读不动了、写口推不动了（后两条都是"持设备的域没了"）
//! ——三条路各留一行读数，日志里那是三件事。孔那一格**装不下就答 `Denied` 且一个字节都不动**
//! （不是截断）⇒ 读缓冲按**载体的界**给一页（见 [`BUF_MAX`]），"装不下"那一支因此**不存在**，
//! 而不是被当成收场吞掉。

use super::console::Console;
use crate::core::line::{Fed, LINE_MAX, Line};
use alloc::vec::Vec;
use protocol::debug;
use runtime::PAGE_SIZE;
use runtime::env::mail::HolePie;

/// 一次从读口读多少字节：**一页**。
///
/// `envcall` 把一条消息卡在 `1..=一页`（`kernel/src/runtime/switcher/envcall/mail.rs` 的 `push`
/// 那一格）⇒ 给一页就**装得下任何一条消息**——与 `driver/rtc/adapt/resident.rs` 备缓冲的口径
/// 同一条。孔那一格装不下不是丢一行，是 `Denied` 且槽一个字节都不动；那一支由这个长度本身
/// 排除，故下面不必再认一次。
const BUF_MAX: usize = PAGE_SIZE;

/// 回显到收场。
///
/// 三条路都是**正常收场**（那台设备没了 ⇒ 回显这件事已经没有可继续的状态），故没有失败那一格
/// ——`main` 那一格因此只报"一次往返都做不成"（找不到控制台）。日志里"读到收场词"、"读口没了"、
/// "写口没了"是三件事。
pub fn run(console: &Console) {
    // 读缓冲**走堆**（与 `driver/rtc/adapt/resident.rs` 备那一页同一手）：任务栈是**定长**的
    // （`kernel/src/layout.rs::TASK_STACK_SIZE`），一页搁在栈上就是那一格里的一大块；而这条栈
    // 上溢出是**当场杀域**（守护页 ⇒ "reserved region access"），不是一句报错。壳里也不该背
    // 一块大缓冲。
    let mut buf: Vec<u8> = Vec::new();
    if buf.try_reserve_exact(BUF_MAX).is_err() {
        debug!("echo: no buffer");
        return;
    }
    buf.resize(BUF_MAX, 0);

    let mut line = Line::new();

    loop {
        // 读那一侧唯一的失败支：读口封了（持设备的域没了）⇒ 收场。
        let Ok(n) = console.rx.pull(&mut buf) else {
            debug!("echo: rx gone");
            return;
        };
        // `n <= buf.len()` 是 `pull` 的契约（返**实际**长度）。破了约是内核那一侧的事，不在
        // 这一层折成"静默收场"——那里该响的是断言，不是一条安静的退场路。
        for &b in &buf[..n] {
            match line.feed(b) {
                Fed::More => {}
                Fed::Line(word) => {
                    if write(&console.tx, word).is_err() {
                        debug!("echo: tx gone");
                        return;
                    }
                }
                Fed::Exit => {
                    debug!("echo: exit");
                    return;
                }
            }
        }
    }
}

/// 把一条字推给控制台（**一次写 = 一条完整的字**：这一条消息就是要写出去的全部字节）。
///
/// **换行本域补**（终端那一侧的约定：`\r` / `\n` / `\r\n` 都算一次断行，见 [`Line::feed`]）——
/// 空行因此也发得出去：一条只有 `\n` 的消息不是空消息（`push` 不收 0 字节）。
///
/// 推不动 = 控制台没了（那枚孔随它死）⇒ 与"读口没了"同一个结局，故返 `Err` 由调用点收场。
fn write(tx: &HolePie, word: &str) -> Result<(), ()> {
    let n = word.len();
    // `Line` 交出来的每一行都不超过 [`LINE_MAX`]（界在它那一侧），故这一格一定搁得下。
    assert!(n <= LINE_MAX);
    let mut one = [0u8; LINE_MAX + 1];
    one[..n].copy_from_slice(word.as_bytes());
    one[n] = b'\n';
    tx.push(&one[..n + 1]).map_err(|_| ())
}
