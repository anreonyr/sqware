//! assemble — **领门闩的域共用的那段客侧装配**：装会话 → 收配给 → 按位次归位。
//!
//! 凡要领门闩的域都走同一条路（今天三台驱动 `router` / `uart` / `rtc`，以及 U 态的房客
//! `lodger`）：
//! 本域那座码头交给**生我者**（= 建本域的那枚线程 = 编排域的装配者），父域按**同一张需求单**
//! 把记录推进来，本域按位次归位。这段机器与"是哪个域"无关，故住在这一级。
//!
//! **照实记**：它住在 `driver` 这一族，而 `lodger` 住 `user`（那一边的判据是特权级，不是角色）
//! ——于是那一档的 bin 反向 `use` 了 `driver` 这一段。名字与住处要不要跟着"领门闩的域"这个
//! 更宽的口径挪一次（`user` 那一档里它是唯一的例外），留给下一刀裁。
//!
//! 发货那一侧是 [`crate::supervisor::service`]（递单 + 推记录）；**字节形状不在这里**
//! ——那是 [`protocol::system::grant`]，与固件回单同一种字节。
//!
//! # 判据落在哪
//!
//! **"要几样"不在这台机器里**：调用方按自己的需求单把数组**解构**出来
//! （`let [Some(a), ..] = slots else { … }`）——缺一格就是装配错，而"该有几格"是收方那张单
//! 的账（`needs::WANTS`）。本模块只保证**回单与单子同序同长**：第 i 条落第 i 格。

use alloc::vec;
use env::{PAIR_LEN, Pair};
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

/// 收一次配给：**回单第 i 条落第 i 格**，返**收到的条数**（读数用）。
///
/// 前置：`slots.len()` = 本域那张单子的条数。
/// 契约：回单与单子**同序同长**——长度不符 ⇒ `Err(E_GRANT)`（这次配给不算，不是"少收几样"）。
/// 坐标与号一起收下（[`Pair`]）：驱动要报线、要开图，都从那一条记录里取，不自己再写一遍。
pub fn receive(slots: &mut [Option<Pair>]) -> Result<usize, usize> {
    let sire = utask::sire().map_err(|_| E_SIRE)?;
    let channel = env::Name::new(RECORDS).map_err(|_| E_UP)?;
    let mut quay = Quay::open(sire);
    quay.seat(channel).map_err(|_| E_UP)?;
    let up = quay.find(channel).ok_or(E_UP)?;
    // 缓冲按本域那张单子备：需求单几条就备几条（发货方不必抄这个数）。
    let mut buf = vec![0u8; PAIR_LEN * slots.len()];
    let n = up.pull(&mut buf, MS).map_err(|_| E_GRANT)?;
    if n != PAIR_LEN * slots.len() {
        // 短了/长了都算这次配给不成立：位置即格，条数对不上就没有"第 i 格"可言。
        return Err(E_GRANT);
    }
    grant::each(&buf[..n], |i, pair| {
        if let Some(cell) = slots.get_mut(i) {
            *cell = Some(pair);
        }
    });
    // 回**条数**（读数是"收到几样"，不是"收到几个字节"）。
    Ok(slots.len())
}
