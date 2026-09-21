//! line::client — **客侧几手**：装一条线泊位、说一声登记、收投递、说一句排空。
//!
//! 客户是**持有那台设备的人**：它从不读线号（泊位就是坐标），只报设备名。

use env::{Name, PieToken, TaskId};
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
    /// 装一条线泊位（记号 [`call::LANE`]）并把登记推给门牌那扇入口，等一格答话。
    ///
    /// `entry` = 树上查来的那扇门（`/device/router` 下驱动族那一块）；对端 = **那扇门的主人**
    /// （`owner`：副本共享同一事实、转手不变）。
    pub fn reserve(entry: PieToken, device: Name, ms: usize) -> Result<Line, Fail> {
        let host = owner_of(entry).ok_or(Fail::Denied)?;
        let mark = Name::new(call::LANE).map_err(|_| Fail::Denied)?;
        let mut quay = Quay::open(host);
        // 本端那一枚交给它（它按"谁开的 + 记号"认下来，往这里投递）。
        quay.seat(mark).map_err(|_| Fail::Denied)?;
        // 回信孔：本端铸一枚、借给它——登记那一答从它回来（单槽的孔只够一个方向）。
        let back = mail::unseal_hole(call::BACK).map_err(|_| Fail::Denied)?;
        port::ship(
            &HolePie::from_token(back),
            host,
            Access::FETCH | Access::STORE,
            Policy::NONE,
        )
        .map_err(|_| Fail::Denied)?;
        HolePie::from_token(entry)
            .push(&call::pack_reserve(device))
            .map_err(|_| Fail::Denied)?;
        let mut one = [0u8; 1];
        let code = match HolePie::from_token(back).pull_timeout(&mut one, ms) {
            Ok(1) => one[0],
            _ => call::BAD,
        };
        if code != call::OK {
            return Err(match code {
                call::TAKEN => Fail::Taken,
                call::UNKNOWN => Fail::Unknown,
                _ => Fail::Denied,
            });
        }
        // 认下它那一枚：它另装了一条泊位的一半，本端写的那一枚从它来。
        quay.claim(host, mark, ms).map_err(|_| Fail::Denied)?;
        Ok(Line { quay })
    }

    /// 收一帧投递（4 字节线号）。`Err(())` = 期限内没等到。
    pub fn recv(&self, buf: &mut [u8], ms: usize) -> Result<u32, ()> {
        let n = self.lane()?.pull(buf, ms)?;
        call::unpack_line(buf.get(..n).ok_or(())?).ok_or(())
    }

    /// 说一句"这一条我处理完了"。**不阻塞**：路由者那一格还压着上一条没取时，就当已经
    /// 说过——它迟早会取到那一条，而这句话说的是**状态**（那一格回闲 + 把线放回），幂等。
    ///
    /// 为什么不能阻塞：路由者投递、客户说排空，两边都是"往对方的单槽里推"。两边都等 ⇒
    /// 谁也回不去取自己那一格，机器当场不动（实测）。堵死的那一条只能是**通知**，
    /// 不能是**移交**——真需要送达的那一路（投递）留在 [`super::core::Lines::deliver`] 上，
    /// 它阻塞，且客户**总会**回到收投递那一格（客户从不堵在说排空上）。
    pub fn exhaust(&self, line: u32) -> Result<(), ()> {
        self.lane()?.try_post(&call::pack_line(line))
    }

    fn lane(&self) -> Result<&Pier, ()> {
        let mark = Name::new(call::LANE).map_err(|_| ())?;
        self.quay.find(mark).ok_or(())
    }
}

/// 那扇门的主人（`Reserve` 的第二格）。树上那一枚是**别人**挂的，故不能按记号认。
fn owner_of(hole: PieToken) -> Option<TaskId> {
    match mail::reserve(hole) {
        Ok((_vestor, owner, _mark)) if owner.get() != 0 => Some(owner),
        _ => None,
    }
}
