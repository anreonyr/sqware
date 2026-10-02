//! 等三源 → 四手各就位。

use super::boot::Up;
use super::event::{bell, desk, exhaust};
use super::sweep;
use env::Wait;
use programs::driver::shared::fail::Fail;
use programs::unit::router::E_ROUTER;

/// **（临时读数）什么算"这一圈慢了"**（毫秒）：一步/一等超过它，这一步就报一行。
const SLOW_MS: usize = 100;

/// **（临时读数）这一步走完了**（毫秒）。
fn ms(from: u64) -> usize {
    (runtime::env::chrono::clock().saturating_sub(from) / 1_000_000) as usize
}

/// **（临时读数）报一行"第几步、进还是出、花了多久"**。全局封顶（一场正常跑一行都不花）。
fn say(name: &str, tag: &str, ms: usize) {
    static N: ::core::sync::atomic::AtomicUsize = ::core::sync::atomic::AtomicUsize::new(0);
    if N.fetch_add(1, ::core::sync::atomic::Ordering::Relaxed) < 400 {
        protocol::debug::put(&alloc::format!("router: step {name} {tag} ms={ms}"));
    }
}

/// **（临时读数）把一步包起来**：`slow` 一旦为真，这一步就**先报"in"再动手**——
/// 于是"卡在某一步里头出不来"那条路上，**"in"就是最后一行**（它就是答案）。
///
/// 为什么不是每圈都报：这一圈每醒一次就走一遍（一场装配几百上千圈），逐圈打会把封顶吃光，
/// 恰恰卡住那几圈看不见。故**只有这一圈已经慢了才开口**（`slow` 由"上一圈/上一等的耗时"或
/// "上一步的耗时"点亮）。这不是看门狗：没有周期扫描、没有额外线程，只有当事人自己按需开口。
fn step<T>(slow: &mut bool, name: &str, f: impl FnOnce() -> T) -> T {
    let t = runtime::env::chrono::clock();
    if *slow {
        say(name, "in", 0);
    }
    let out = f();
    let took = ms(t);
    if took >= SLOW_MS {
        *slow = true;
    }
    if *slow {
        say(name, "out", took);
    }
    out
}

/// 常驻：**一只组等两个源**（加上门牌，共三个）
/// 由内核**永久持有**，`platform/devices.rs::IRQ`——它是一格防御，不是读数）
pub fn run(up: &mut Up) -> Result<(), Fail> {
    loop {
        let mut slow = false;
        // **等到有事件**：三样（铃 / 门上有人 / 客人的排空）都可等地，醒来就说明有一格有事。
        // **纯事件（Wait::Forever），没有兜底的一拍**：铃的根在铃那一侧——空闲核不进外部 trap，
        // `raise_irq` 在它身上没有调用点（见 `kernel/src/work/room/scheduler/core/fetch.rs`
        // 的空闲循环）。根修在那里，`SEIP` 能挂的那两条长驻态各有振铃点之后这一拍就是多余的：
        // 铃一定响，醒来 `claim`+`hush` 即到。
        let t_wait = runtime::env::chrono::clock();
        let awaited = up.pile.await_(Wait::Forever);
        let waited = ms(t_wait);
        if waited >= SLOW_MS {
            slow = true;
            say("await", "out", waited);
        }
        match awaited {
            Ok(Some(_)) => {}
            // 挂起过（不是期限）：照样往下走一遍——`claim` 领到空就什么也不做。
            Ok(None) => {}
            Err(_) => return Err(Fail::at(E_ROUTER, "bell")),
        }
        // 逐客：**每次醒来扫一遍有主的那些条**——主人没了就拆线 + 空出格子。放在最前：
        step(&mut slow, "sweep", || {
            sweep::run(&mut up.lines, &up.plic, &up.pile)
        });
        // 排空：客人说一句"这一条我排空了" ⇒ 那一格回闲 + **把线放回去**（事件，不是节拍）。
        // **先取排空，再登记**：登记会把新的一条线接上，紧接着到来的那一枚中断才不漏。
        step(&mut slow, "drain", || {
            exhaust::drain(&mut up.lines, &up.plic)
        });
        // 门上：非阻塞地把手上那些都取走（登记）。一份**够大**的缓冲接一条消息——
        // 「取不出也丢不掉」（取不出的那条会留在孔上，把后面正经的问堵在门外）。
        step(&mut slow, "entry", || {
            while let Ok((n, from)) = up.entry.pull(&mut up.buf, Wait::POLL) {
                desk::serve(
                    &mut up.lines,
                    &up.plic,
                    from,
                    &up.buf[..n],
                    &up.pile,
                    &mut up.replies,
                );
            }
        });
        step(&mut slow, "bell", || bell::ring(&mut up.lines, &up.plic));
        // 应铃：清掉那一位并让内核**立即**重开本 hart 的闸门。**无条件**做——
        // 没响时它答 `Busy`（幂等），而少做一次就是闸门永久关着。
        step(&mut slow, "hush", || {
            let _ = up.bell.hush();
        });
    }
}
