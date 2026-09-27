//! line::client — **客侧几手**：占住一条线泊位、说一声登记、收投递、说一句排空。
//!
//! 客户是**持有那台设备的人**：它从不读线号（泊位就是坐标），只报**那一段区**。

use env::Wait;
use env::{Mark, PieToken};
use env::Key;
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail::{self, HolePie};

use super::frame::Fail;
use super::frame;
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

impl Line {
    /// 占住这一格（记号 [`frame::LANE`]）并把登记推给门牌那扇入口，等一格答话。
    ///
    /// `entry` = 树上查来的那扇门（`/device/router` 下驱动族那一块）；对端 = **那扇门的主人**
    /// （`owner`：副本共享同一事实、转手不变）。
    ///
    /// **失败那一趟两边都收干净**：本端铸出去的那一枚（`pair` 是 [`Held`]，三条 `return` 上
    /// 各自放下）与本趟借出去的那枚回信孔。两枚都**不在任何账上**——账里根本没有这一格，
    /// 故此后没人会替它收，而路由者那侧**收不了别人的表**（它只放得下自己表里的那一枚），
    /// 故这一侧自己收干净。不这么做的话，一个会重试的客户每失败一次就在自己表里多留两枚，
    /// 直到它退场（读数见 `programs/src/driver/router/adapt/desk.rs` 那一格 `pies=`）。
    pub fn occupy(entry: PieToken, key: Key, millis: Wait) -> Result<Line, Fail> {
        let host = establish::opened_by(entry).ok_or(Fail::Denied)?;
        // 本端那一枚先铸出来交给它（它按"谁开的 + 记号"认下来，往这里投递）。**这一步不等对端
        // 那一枚**：对端要到它读过登记那一句之后才装它那一半（次序是契约的一半，见下面 `claim`）。
        // **有主地建**：那一格"有主"由类型说出来——`Held(endpoint(..)?)`（没有 `hold` 那一手：
        // 它只是这一个字面量）。这一条线归本端持有，`Line` 落出作用域即放下；失败那几趟
        // 也由它的 `Drop` 代劳（下面三处 `return` 一个字都不用写）。
        let mut pair = Held(
            establish::endpoint(host, Mark::of(frame::LANE), Wait::POLL)
                .map_err(|_| Fail::Denied)?,
        );
        // 回信孔：本端铸一枚、借给它——登记那一答从它回来（单槽的孔只够一个方向）。
        let back = mail::unseal_hole(frame::BACK_MARK).map_err(|_| Fail::Denied)?;
        // 从这一手起，每一次失败都要收干净（那枚回信孔 + 这条线）——**线由 `pair` 的 `Drop`
        // 收**（放的是本端铸的那一枚），回信孔由本函数收（它不是本端铸的）。
        let sent = port::ship(
            &HolePie::from_token(back),
            host,
            Access::FETCH | Access::STORE,
            Policy::NONE,
        )
        .map_err(|_| ())
        .and_then(|_| {
            // 登记那一句：**走 `Sender`**（这一族一问只有一形：动作码 ＋ 坐标）——装与发都不在这一
            // 层写字节（缓冲在这一帧的栈上：这一形定长 [`frame::Occupy::LEN`]）。
            Sender::<frame::Occupy>::from_token(entry)
                .send(frame::Occupy::of(key), Wait::Forever)
                .map_err(|_| ())
        });
        if sent.is_err() {
            let _ = mail::release(back);
            return Err(Fail::Denied);
        }
        let mut one = [0u8; 1];
        let code = match HolePie::from_token(back).pull_timeout(&mut one, millis) {
            Ok(1) => one[0],
            _ => frame::BAD,
        };
        // 答话到手 ⇒ 这一枚回信孔这一趟就用完了：**当场放下**（一问一答一个往返）。放下的是本端
        // 这一份，路由者那一份由它自己放。
        let _ = mail::release(back);
        if code != frame::OK {
            return Err(match code {
                frame::TAKEN => Fail::Taken,
                frame::UNKNOWN => Fail::Unknown,
                _ => Fail::Denied,
            });
        }
        // 认下它那一枚：它另装了一条泊位的一半，本端写的那一枚从它来。
        if !pair.claim(host, Mark::of(frame::LANE), millis) {
            return Err(Fail::Denied);
        }
        Ok(Line { pair })
    }

    /// 收一帧投递。`Err(())` = 期限内没等到。
    ///
    /// **帧里没有线号**（线在泊位里，见 [`super`]）：这一手对客户就是"我那一格有事"。
    ///
    /// **这一格是裸字节**（投递那一帧只有一个动作码），不是一族那种报 ⇒ 走裸孔，不套手柄。
    pub fn receive(&self, millis: Wait) -> Result<(), ()> {
        let mut one = [0u8; 1];
        match HolePie::from_token(self.pair.rx()).pull_timeout(&mut one, millis) {
            Ok(_) => Ok(()),
            Err(_) => Err(()),
        }
    }

    /// 说一句"这一条我处理完了"。**不阻塞**：路由者那一格还压着上一条没取时，就当已经
    /// 说过——它迟早会取到那一条，而这句话说的是**状态**（那一格回闲 + 把线放回），幂等。
    ///
    /// 为什么不能阻塞：路由者投递、客户说排空，两边都是"往对方的单槽里推"。两边都等 ⇒
    /// 谁也回不去取自己那一格，机器当场不动（实测）。堵死的那一条只能是**通知**，
    /// 不能是**移交**——真需要送达的那一路（投递）留在 投递那一手（`Lines::deliver`，住路由者那一侧） 上，
    /// 它阻塞，且客户**总会**回到收投递那一格（客户从不堵在说排空上）。
    pub fn exhaust(&self) -> Result<(), ()> {
        let Some(tx) = self.pair.tx() else {
            return Err(());
        };
        // 单次尝试（`mail::push` 不挂起）：槽满当场答 `Err`——原 `try_post` 的那一格。
        let note = [frame::NOTE];
        mail::push(tx, note.as_ptr(), note.len()).map_err(|_| ())
    }

    /// 本端读的那一枚（**挂进组**用：一台驱动要同时等"线上有投递"与"门上有人"）。
    ///
    /// 与 [`Line::receive`] 读的是同一枚——组等的是**就绪**，取消息仍走 `receive`。
    pub fn hole(&self) -> Result<PieToken, ()> {
        Ok(self.pair.rx())
    }
}
