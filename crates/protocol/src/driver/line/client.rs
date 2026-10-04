//! 占住一条线泊位、说一声登记、收投递、说一句排空。
//! 客户是**持有那台设备的人**：它**不自己算线号**——那个数来自认领那一答的契
//! （Deed，区→线的权威在设备账那一台），本层只把它原样报上来。

use env::{Wait, HoleDir, Mark, PieToken};
use runtime::core::res::port::{self, Access, Policy};

use super::frame;
use super::frame::Fail;
use crate::communication::hand::Sender;
use crate::communication::session::establish::{self, Held};
use env::pie;
use runtime::core::res::pie::{HolePie};

/// 客户手里那一条线：一对孔（本端读投递、写排空）
/// **归本端持有**（Held）：`Line` 落出作用域就是"这条线我不要了"——本端那一枚随 `Drop`
/// Endpoint（那一类归域、放不下）
pub struct Line {
    pair: Held,
}

// Line::occupy 把**七条完全不同的成因**折成同一个 Fail::Denied（线上那张表里 `DENIED` 也是
// 3）⇒ 只看客侧那一格码，分不出"门忙/没答"与"孔不够/认不下对端"。这两格记下**最后一条出口的
pub static OCCUPY_DENY: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);
pub static OCCUPY_CODE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

fn deny(cause: u8, code: u8) -> Fail {
    use core::sync::atomic::Ordering;
    OCCUPY_DENY.store(cause, Ordering::Relaxed);
    OCCUPY_CODE.store(code, Ordering::Relaxed);
    Fail::Denied
}

impl Line {
    pub fn occupy(entry: PieToken, line: u32, millis: Wait) -> Result<Line, Fail> {
        let Some(host) = establish::opened_by(entry) else {
            return Err(deny(1, 0));
        };
        // 那一枚**：对端要到它读过登记那一句之后才装它那一半（次序是契约的一半，见下面 `claim`）。
        // **有主地建**：那一格"有主"由类型说出来——`Held(endpoint(..)?)`（没有 `hold` 那一手：
        // 它只是这一个字面量）。这一条线归本端持有，`Line` 落出作用域即放下；失败那几趟
        // 也由它的 `Drop` 代劳（下面三处 `return` 一个字都不用写）。
        let mut pair = match establish::endpoint(host, Mark::of(frame::LANE), Wait::POLL) {
            Ok(ep) => Held(ep),
            Err(_) => return Err(deny(2, 0)),
        };
        // 回信孔：本端铸一枚、借给它——登记那一答从它回来（单手的孔只够一个方向）。
        let back = match pie::unseal_hole(frame::BACK_MARK) {
            Ok(back) => back,
            Err(_) => return Err(deny(3, 0)),
        };
        // 收**（放的是本端铸的那一枚），回信孔由本函数收（它不是本端铸的）。
        if port::ship(
            back,
            host,
            Access::FETCH | Access::STORE,
            Policy::NONE,
        )
        .is_err()
        {
            // "这只手被取走"（Sender::Drop），而它等的这一枚只有我手里这一份。
            let _ = pie::seal(back);
            let _ = pie::release(back);
            return Err(deny(4, 0));
        }
        // 层写字节。**递出即返回**：等它下线由这一枚 `Sender` 担着（`reclaim`，`Drop` 兜底）——
        // 推完就落地等于"等对面来取"，会卡住回话。
        let mut out = Sender::<frame::Occupy>::from_token(entry);
        let budget = match millis {
            Wait::POLL => 1,
            Wait::AtMost(ms) => ms,
            Wait::Forever => 1000,
        };
        let mut spent = 0usize;
        while out.send(frame::Occupy::of(line)).is_err() {
            spent += 1;
            if spent >= budget {
                break;
            }
            let _ = runtime::core::task::sleep(core::time::Duration::from_millis(1));
        }
        if spent >= budget {
            // "这只手被取走"（Sender::Drop），而它等的这一枚只有我手里这一份。
            let _ = pie::seal(back);
            let _ = pie::release(back);
            return Err(deny(5, 0));
        }
        let mut one = [0u8; 1];
        let code = match HolePie::from_token(back).pull(&mut one, millis) {
            Ok((1, _)) => one[0],
            _ => frame::BAD,
        };
        let _ = out.reclaim();
        // 这一份，路由者那一份由它自己放。
        // "这只手被取走"（Sender::Drop），而它等的这一枚只有我手里这一份。
        let _ = pie::seal(back);
        let _ = pie::release(back);
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

    /// 收一帧投递。`Err(())` = 期限内没等到
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

    /// 说一句"这一条我处理完了"。**不阻塞**：已经在响就当也说了——它迟早会应掉那一位
    /// 而这句话说的是**状态**（那一格回闲 + 把线放回），幂等
    /// **为什么不能阻塞**：路由者投递、客户说排空，两边都是"往对方那一格上说一句"。两边都等 ⇒
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

    /// 本端读的那一枚（**挂进组**用：一台驱动要同时等"线上有投递"与"门上有人"）
    /// 与 Line::receive 读的是同一枚——组等的是**就绪**，取消息仍走 `receive`
    pub fn hole(&self) -> Result<PieToken, ()> {
        Ok(self.pair.rx())
    }
}
