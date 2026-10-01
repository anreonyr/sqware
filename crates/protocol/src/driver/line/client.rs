//! line::client — **客侧几手**：占住一条线泊位、说一声登记、收投递、说一句排空。
//!
//! 客户是**持有那台设备的人**：它**不自己算线号**——那个数来自认领那一答的契
//! （[`Deed`](crate::driver::hub::Deed)，区→线的权威在设备账那一台），本层只把它原样报上来。

use env::Wait;
use env::{HoleDir, Mark, PieToken};
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail::{self, HolePie};

use super::frame;
use super::frame::Fail;
use crate::communication::establish::{self, Held};
use crate::communication::sender::Sender;

/// 客户手里那一条线：一对孔（本端读投递、写排空）。
///
/// **归本端持有**（[`Held`]）：`Line` 落出作用域就是"这条线我不要了"——本端那一枚随 `Drop`
/// 放下，**一处也不用记**。这一段关系的寿命就是"我拿着这条 Line"，故它不走
/// [`Endpoint`](crate::communication::establish::Endpoint)（那一类归域、放不下）。
pub struct Line {
    pair: Held,
}

// ── 读数：**这一趟折在哪一条出口上**（七条折成同一个 `Denied`，那一格码分不出来）──────
//
// `Line::occupy` 把**七条完全不同的成因**折成同一个 `Fail::Denied`（线上那张表里 `DENIED` 也是
// 3）⇒ 只看客侧那一格码，分不出"门忙/没答"与"孔不够/认不下对端"。这两格记下**最后一条出口的
// 号**（**决策之前一个字节都不落**，故不改这条路的时候），由客人（`harness/src/lodger.rs`）在
// 它那条判据上读出来。**为什么留着**：`scene root` 今天仍有约四分之一的跑红，红的签名正是
// "房客第一趟登记拿到 `Denied`"（见 `programs/src/driver/router/adapt/desk.rs` 的照实记），
// 下面这两格就是下一次读它的第一手。
//
//   1 门牌读不出开者（`opened_by`）      5 登记那一句推不出去（`Sender::send`）
//   2 铸/交不出本端那一半（`endpoint`）  6 路由者答的不是 `OK`（第二格记它答的**原码**）
//   3 铸不出回信孔（`unseal_hole`）      7 答话到手、可认不下对端那一半（`claim`）
//   4 回信孔交不出去（`port::ship`）
pub static OCCUPY_DENY: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);
pub static OCCUPY_CODE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

fn deny(cause: u8, code: u8) -> Fail {
    use core::sync::atomic::Ordering;
    OCCUPY_DENY.store(cause, Ordering::Relaxed);
    OCCUPY_CODE.store(code, Ordering::Relaxed);
    Fail::Denied
}

impl Line {
    /// 占住这一格（记号 [`frame::LANE`]）并把登记推给门牌那扇入口，等一格答话。
    ///
    /// `entry` = 树上查来的那扇门（`/svc/drv/router` 下驱动族那一块）；对端 = **那扇门的主人**
    /// （`owner`：副本共享同一事实、转手不变）。
    ///
    /// **失败那一趟两边都收干净**：本端铸出去的那一枚（`pair` 是 [`Held`]，三条 `return` 上
    /// 各自放下）与本趟借出去的那枚回信孔。两枚都**不在任何账上**——账里根本没有这一格，
    /// 故此后没人会替它收，而路由者那侧**收不了别人的表**（它只放得下自己表里的那一枚），
    /// 故这一侧自己收干净。不这么做的话，一个会重试的客户每失败一次就在自己表里多留两枚，
    /// 直到它退场（读数见 `programs/src/driver/router/adapt/desk.rs` 那一格 `pies=`）。
    ///
    /// **荷载是线号不是坐标**（照实记，见 [`frame::Occupy`]）：区 → 线那条权威在**设备账**
    /// 那一台（认领那一答的契里带着线号），本手只是把那个数原样报上去。
    pub fn occupy(entry: PieToken, line: u32, millis: Wait) -> Result<Line, Fail> {
        let Some(host) = establish::opened_by(entry) else {
            return Err(deny(1, 0));
        };
        // 本端那一枚先铸出来交给它（它按"谁开的 + 记号"认下来，往这里投递）。**这一步不等对端
        // 那一枚**：对端要到它读过登记那一句之后才装它那一半（次序是契约的一半，见下面 `claim`）。
        // **有主地建**：那一格"有主"由类型说出来——`Held(endpoint(..)?)`（没有 `hold` 那一手：
        // 它只是这一个字面量）。这一条线归本端持有，`Line` 落出作用域即放下；失败那几趟
        // 也由它的 `Drop` 代劳（下面三处 `return` 一个字都不用写）。
        let mut pair = match establish::endpoint(host, Mark::of(frame::LANE), Wait::POLL) {
            Ok(ep) => Held(ep),
            Err(_) => return Err(deny(2, 0)),
        };
        // 回信孔：本端铸一枚、借给它——登记那一答从它回来（单手的孔只够一个方向）。
        let back = match mail::unseal_hole(frame::BACK_MARK) {
            Ok(back) => back,
            Err(_) => return Err(deny(3, 0)),
        };
        // 从这一手起，每一次失败都要收干净（那枚回信孔 + 这条线）——**线由 `pair` 的 `Drop`
        // 收**（放的是本端铸的那一枚），回信孔由本函数收（它不是本端铸的）。
        if port::ship(
            &HolePie::from_token(back),
            host,
            Access::FETCH | Access::STORE,
            Policy::NONE,
        )
        .is_err()
        {
            // **读者结清（B-a）**：这一枚是本端铸的、只活这一趟 ⇒ **先封印、再放下**——另一头若还在等
            // "这只手被取走"（`Sender::Drop`），而它等的这一枚只有我手里这一份。
            let _ = mail::seal(back);
            let _ = mail::release(back);
            return Err(deny(4, 0));
        }
        // 登记那一句：**走 `Sender`**（这一族一问只有一形：动作码 ＋ 线号）——装与发都不在这一
        // 层写字节。**递出即返回**：等它下线由这一枚 `Sender` 担着（`reclaim`，`Drop` 兜底）——
        // 推完就落地等于"等对面来取"，会卡住回话。
        let mut out = Sender::<frame::Occupy>::from_token(entry);
        if out.send(frame::Occupy::of(line)).is_err() {
            // **读者结清（B-a）**：这一枚是本端铸的、只活这一趟 ⇒ **先封印、再放下**——另一头若还在等
            // "这只手被取走"（`Sender::Drop`），而它等的这一枚只有我手里这一份。
            let _ = mail::seal(back);
            let _ = mail::release(back);
            return Err(deny(5, 0));
        }
        let mut one = [0u8; 1];
        let code = match HolePie::from_token(back).pull(&mut one, millis) {
            Ok((1, _)) => one[0],
            _ => frame::BAD,
        };
        // 答话到手（或这一趟判了失败）⇒ 把那一手收口：对面取走了是零代价，没取走就等它取。
        let _ = out.reclaim();
        // 答话到手 ⇒ 这一枚回信孔这一趟就用完了：**当场放下**（一问一答一个往返）。放下的是本端
        // 这一份，路由者那一份由它自己放。
        // **读者结清（B-a）**：这一枚是本端铸的、只活这一趟 ⇒ **先封印、再放下**——另一头若还在等
        // "这只手被取走"（`Sender::Drop`），而它等的这一枚只有我手里这一份。
        let _ = mail::seal(back);
        let _ = mail::release(back);
        if code != frame::OK {
            deny(6, code);
            return Err(match code {
                frame::TAKEN => Fail::Taken,
                frame::UNKNOWN => Fail::Unknown,
                _ => Fail::Denied,
            });
        }
        // 认下它那一枚：它另装了一条泊位的一半，本端写的那一枚从它来。
        if !pair.claim(host, Mark::of(frame::LANE), millis) {
            return Err(deny(7, 0));
        }
        Ok(Line { pair })
    }

    /// 收一帧投递。`Err(())` = 期限内没等到。
    ///
    /// **帧里没有线号**（线在泊位里，见 [`super`]）：这一手对客户就是"我那一格有事"。
    ///
    /// **这一格是孔上那一位**（不是一族那种报）⇒ 走裸孔，不套手柄。两拍与旧孔时代同形：
    /// **等**（`wait`）＋ **应**（`hush`）——`Wait::POLL` 就是"只看一眼"，一次也不挂起。
    pub fn receive(&self, millis: Wait) -> Result<(), ()> {
        let rx = self.pair.rx();
        if HolePie::from_token(rx)
            .wait(HoleDir::Pull, millis)
            .map_err(|_| ())?
        {
            HolePie::from_token(rx).hush().map_err(|_| ())
        } else {
            Err(())
        }
    }

    /// 说一句"这一条我处理完了"。**不阻塞**：已经在响就当也说了——它迟早会应掉那一位，
    /// 而这句话说的是**状态**（那一格回闲 + 把线放回），幂等。
    ///
    /// **为什么不能阻塞**：路由者投递、客户说排空，两边都是"往对方那一格上说一句"。两边都等 ⇒
    /// 谁也回不去取自己那一格，机器当场不动（**实测**——旧孔时代它真的发生过）。堵死的那一条
    /// 只能是**通知**，不能是**移交**；本刀把这两条通知都换成**位**（`ring` 置位即返，
    /// 从不睡），那条互锁于是**结构上不可能**。
    pub fn exhaust(&self) -> Result<(), ()> {
        let Some(tx) = self.pair.tx() else {
            return Err(());
        };
        // 置位即返：已响 = "这一条我处理完了"这件**状态**已经有了 ⇒ 也算说过。
        match HolePie::from_token(tx).ring() {
            Ok(()) => Ok(()),
            Err(e) if e.source.is_busy() => Ok(()),
            Err(_) => Err(()),
        }
    }

    /// 本端读的那一枚（**挂进组**用：一台驱动要同时等"线上有投递"与"门上有人"）。
    ///
    /// 与 [`Line::receive`] 读的是同一枚——组等的是**就绪**，取消息仍走 `receive`。
    pub fn hole(&self) -> Result<PieToken, ()> {
        Ok(self.pair.rx())
    }
}
