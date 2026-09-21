//! assemble — **驱动域的客侧装配**：装会话 → 收配给 → 按 `Slot` 归位。
//!
//! 两台驱动领门闩走的是同一条路：本域那座码头交给**生我者**（= 建本域的那枚线程 = 编排域的
//! 装配者），父域按**同一张需求单**把记录推进来，本域按自己的 `Slot` 归位。这段机器与
//! "是哪台驱动"无关，故住在驱动这一族这一级。
//!
//! 发货那一侧是 [`crate::supervisor::service`]（递单 + 推记录）；**字节形状不在这里**
//! ——那是 [`protocol::system::grant`]，与固件回单同一种字节。
//!
//! # 判据落在哪
//!
//! **"要几样"不在这台机器里**：调用方按自己的需求单把数组**解构**出来
//! （`let [Some(a), ..] = slots else { … }`）——缺一格就是装配错，而"该有几格"是收方那张单
//! 的账（`needs::WANTS`）。本模块只保证"名字翻得出的都归了位"。

use alloc::vec;
use env::PieToken;
use protocol::session::Quay;
use protocol::system::grant;
use runtime::env::unit as utask;

/// 收记录那条通道的名字——**两端同一个**（装配单的 `channels` 里也写的它）。
pub const RECORDS: &str = "records";

/// 装配期等配给的上限（毫秒）。**必须有界**：父域死在递单之前时本域不能陪着挂死。
pub const MS: usize = 1000;

/// 装配失败编号，**这一族共用**：指"死在装配的哪一步"（各驱动自己那几格从 4 起）。
pub const E_SIRE: usize = 1;
pub const E_UP: usize = 2;
pub const E_GRANT: usize = 3;

/// 收一次配给：按 `slot_of` 把每条记录归位，返**收到的条数**（读数用）。
///
/// 父域按同一张单子发货 ⇒ 条数与单子同长；归不上的名字由 [`grant::unpack`] 直接跳过。
pub fn receive(
    slots: &mut [Option<PieToken>],
    slot_of: impl Fn(&str) -> Option<usize>,
) -> Result<usize, usize> {
    let sire = utask::sire().map_err(|_| E_SIRE)?;
    let channel = env::Name::new(RECORDS).map_err(|_| E_UP)?;
    let mut quay = Quay::open(sire);
    quay.seat(channel).map_err(|_| E_UP)?;
    let up = quay.find(channel).ok_or(E_UP)?;
    // 缓冲按本域那张单子备：需求单几条就备几条（发货方不必抄这个数）。
    let mut buf = vec![0u8; env::PAIR_LEN * slots.len()];
    let n = up.pull(&mut buf, MS).map_err(|_| E_GRANT)?;
    grant::unpack(&buf[..n], slot_of, |slot, token| {
        if let Some(cell) = slots.get_mut(slot) {
            *cell = Some(token);
        }
    });
    // 回**条数**（读数是"收到几样"，不是"收到几个字节"）。
    Ok(n / env::PAIR_LEN)
}
