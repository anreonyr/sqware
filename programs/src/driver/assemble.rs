//! assemble — **领门闩的域共用的那段客侧装配**：装会话 → 收配给 → 按位次归位。
//!
//! 凡要领门闩的域都走同一条路（今天三台驱动 `router` / `uart` / `rtc`，以及 U 态的房客
//! `lodger`）：
//! 本域那条 `records` 路交给**生我者**（= 建本域的那枚线程 = 编排域的装配者），父域按**同一张需求单**
//! 把记录推进来，本域按位次归位。这段机器与"是哪个域"无关，故住在这一级。
//!
//! **照实记**：它住在 `driver` 这一族，而 `lodger` 住 `user`（那一边的判据是特权级，不是角色）
//! ——于是那一档的 bin 反向 `use` 了 `driver` 这一段。名字与住处要不要跟着"领门闩的域"这个
//! 更宽的口径挪一次（`user` 那一档里它是唯一的例外），留给下一刀裁。
//!
//! 发货那一侧是 [`crate::system::control`]（递单 + 推记录）；**字节形状不在这里**
//! ——那是 [`protocol::system::grant`]，与固件回单同一种字节。
//!
//! # 判据落在哪
//!
//! **"要几样"只由收方那张单子说**：[`take`] 的 `N` 就是它的长度，调用方按它**解构**
//! （`let [serial] = assemble::take::<{ WANTS.len() }>()?;`）——缺一格就是装配错，而那张单
//! 今天住 `programs::program`。本模块只保证**回单与单子同序同长**：第 i 条落第 i 格。
//! **`Option` 数组与"第 i 格"不再出现在驱动里**（照实记见 [`take`]）。

use alloc::vec;
use env::Wait;
use env::{PAIR_LEN, Pair};
use protocol::communication::establish;
use protocol::system::grant;
use runtime::env::mail::HolePie;
use runtime::env::unit as utask;

use crate::program::Died;

/// 收记录那条通道的名字——**两端同一个**（装配表里 `Setup::Channel("records")` 也写的它）。
pub const RECORDS: &str = "records";

/// 装配期等配给的上限（毫秒）。**必须有界**：父域死在递单之前时本域不能陪着挂死。
pub const MS: usize = 1000;

/// 装配失败编号，**这一族共用**：指"死在装配的哪一步"（各驱动自己那几步报的是装配表里那一号
/// ——见 [`crate::driver::fail`]；这一族这两个号照旧原样带过）。
///
/// **照实记（`E_SIRE = 1` 已撤）**：那一格是 `sire()` 失败时的号，而 `UnitCall::Sire`
/// 恒写 id（见 `env::ecall::EnvResult` 的注）⇒ `1` 今天无人产生。**号不回填**：
/// 旧 trace 里 `1` 照旧读作 E_SIRE。
pub const E_UP: usize = 2;
pub const E_GRANT: usize = 3;

/// 收一次配给：**回单第 i 条落第 i 格**——`N` 就是本域那张单子的条数。
///
/// 契约：回单与单子**同序同长**——长度不符 ⇒ `Err(E_GRANT)`（这次配给不算，不是"少收几样"）。
/// 坐标与号一起收下（[`Pair`]）：驱动要报线、要开图，都从那一条记录里取，不自己再写一遍。
///
/// **照实记（从前是 `receive(&mut [Option<Pair>])`）**：旧签名把"收几样"提前摊在调用方——
/// 每一处都要先 `let mut slots = [None; WANTS.len()];`，再逐格 pattern match `Some(..)`，
/// 于是 `Option` 数组与"第 i 格"这个**装配内部表示**漏进了驱动里。今天一次收齐：
/// `let [serial] = assemble::take::<{ WANTS.len() }>()?;`——缺格/条数不符仍是同一个 `E_GRANT`，
/// 而"有几格"只由收方那张单子说。
pub fn take<const N: usize>() -> Result<[Pair; N], Died> {
    let sire = utask::sire();
    let channel = env::Name::new(RECORDS).map_err(|_| E_UP)?;
    // 一手就是"两头都装"：铸本域那一枚（刻 `records` 的记号）交给生我者，并顺手试认它那一枚
    // （`POLL` = 不等：**配给走的是本域那一枚**——`pull` 收的就是它；对端那一枚本域用不上）。
    // 父域在放行本域**之前**已经 `connect` 过（`Control::connect`），故它那一枚通常当场到手。
    let up = establish::endpoint(sire, env::Mark::of(channel.as_str()), Wait::POLL)
        .map_err(|_| E_UP)?;
    // 缓冲按本域那张单子备：需求单几条就备几条（发货方不必抄这个数）。
    let mut buf = vec![0u8; PAIR_LEN * N];
    let n = HolePie::from_token(up.rx())
        .pull_timeout(&mut buf, Wait::AtMost(MS))
        .map_err(|_| E_GRANT)?;
    if n != PAIR_LEN * N {
        // 短了/长了都算这次配给不成立：位置即格，条数对不上就没有"第 i 格"可言。
        return Err(E_GRANT);
    }
    let mut out = [Pair::NONE; N];
    grant::each(&buf[..n], |i, pair| {
        if let Some(cell) = out.get_mut(i) {
            *cell = pair;
        }
    });
    Ok(out)
}
