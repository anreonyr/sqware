//! line::client — **客侧几手**：占住一条线泊位、说一声登记、收投递、说一句排空。
//!
//! 客户是**持有那台设备的人**：它从不读线号（泊位就是坐标），只报设备名。

use env::Mark;
use env::{Name, PieToken};
use runtime::core::port::{self, Access, Policy};
use runtime::env::mail::{self, HolePie};

use super::call;
use super::core::Fail;
use crate::session::{Pier, Quay};

/// 客户手里那一条线：一条泊位（本端读投递、写排空）。
pub struct Line {
    quay: Quay,
}

impl Line {
    /// 占住这一格（记号 [`call::LANE`]）并把登记推给门牌那扇入口，等一格答话。
    ///
    /// `entry` = 树上查来的那扇门（`/device/router` 下驱动族那一块）；对端 = **那扇门的主人**
    /// （`owner`：副本共享同一事实、转手不变）。
    ///
    /// **失败那一趟两边都收干净**：本端 `seat` 出去的那一枚（[`Quay::shut`]）与本趟借出去的
    /// 那枚回信孔。两枚都**不在任何账上**——账里根本没有这一格，故此后没人会替它收，而路由者
    /// 那侧**收不了别人的表**（它只放得下自己表里的副本，见 `driver/router` 的 `drop_lane`）。
    /// 不这么做的话，一个会重试的客户每失败一次就在自己表里多留两枚，直到它退场。
    pub fn occupy(entry: PieToken, device: Name, millis: usize) -> Result<Line, Fail> {
        let host = crate::session::call::opened_by(entry).ok_or(Fail::Denied)?;
        let mark = Name::new(call::LANE).map_err(|_| Fail::Denied)?;
        let mut quay = Quay::open(host);
        // 本端那一枚交给它（它按"谁开的 + 记号"认下来，往这里投递）。
        quay.seat(mark).map_err(|_| Fail::Denied)?;
        // 回信孔：本端铸一枚、借给它——登记那一答从它回来（单槽的孔只够一个方向）。
        let back = mail::unseal_hole(call::BACK_MARK).map_err(|_| Fail::Denied)?;
        // 从这一手起，每一次失败都要收干净（那枚回信孔 + 这条泊位）。
        let sent = port::ship(
            &HolePie::from_token(back),
            host,
            Access::FETCH | Access::STORE,
            Policy::NONE,
        )
        .and_then(|_| HolePie::from_token(entry).push(&call::pack_occupy(device)));
        if sent.is_err() {
            let _ = mail::release(back);
            quay.shut();
            return Err(Fail::Denied);
        }
        let mut one = [0u8; 1];
        let code = match HolePie::from_token(back).pull_timeout(&mut one, millis) {
            Ok(1) => one[0],
            _ => call::BAD,
        };
        // 答话到手 ⇒ 这一枚回信孔这一趟就用完了：**当场放下**（一问一答一个往返，见
        // `protocol::session` 事实 2）。放下的是本端这一份，路由者那一份由它自己放。
        let _ = mail::release(back);
        if code != call::OK {
            quay.shut();
            return Err(match code {
                call::TAKEN => Fail::Taken,
                call::UNKNOWN => Fail::Unknown,
                _ => Fail::Denied,
            });
        }
        // 认下它那一枚：它另装了一条泊位的一半，本端写的那一枚从它来。
        if quay.claim(host, Mark::of(call::LANE), millis).is_err() {
            quay.shut();
            return Err(Fail::Denied);
        }
        Ok(Line { quay })
    }

    /// 收一帧投递。`Err(())` = 期限内没等到。
    ///
    /// **帧里没有线号**（线在泊位里，见 [`super::mod`]）：这一手对客户就是"我那一格有事"。
    pub fn receive(&self, millis: usize) -> Result<(), ()> {
        let mut one = [0u8; 1];
        self.lane()?.pull(&mut one, millis).map(|_| ())
    }

    /// 说一句"这一条我处理完了"。**不阻塞**：路由者那一格还压着上一条没取时，就当已经
    /// 说过——它迟早会取到那一条，而这句话说的是**状态**（那一格回闲 + 把线放回），幂等。
    ///
    /// 为什么不能阻塞：路由者投递、客户说排空，两边都是"往对方的单槽里推"。两边都等 ⇒
    /// 谁也回不去取自己那一格，机器当场不动（实测）。堵死的那一条只能是**通知**，
    /// 不能是**移交**——真需要送达的那一路（投递）留在 [`super::core::Lines::deliver`] 上，
    /// 它阻塞，且客户**总会**回到收投递那一格（客户从不堵在说排空上）。
    pub fn exhaust(&self) -> Result<(), ()> {
        self.lane()?.try_post(&[call::NOTE])
    }

    /// 本端读的那一枚（**挂进组**用：一台驱动要同时等"线上有投递"与"门上有人"）。
    ///
    /// 与 [`Line::receive`] 读的是同一枚——组等的是**就绪**，取消息仍走 `receive`。
    pub fn hole(&self) -> Result<PieToken, ()> {
        Ok(self.lane()?.hole())
    }

    fn lane(&self) -> Result<&Pier, ()> {
        let mark = Name::new(call::LANE).map_err(|_| ())?;
        self.quay.find(mark).ok_or(())
    }
}
