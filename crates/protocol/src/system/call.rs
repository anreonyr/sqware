//! 适配 —— 把 [`service`](super::service) 的判定落到运行时那几件上。
//!
//! 每个函数只做一件事：**转发一次**，再把内核答的事实（或错误码）翻成核心认的字。
//! 判断一律不在这里（那在 `service.rs`）。
//!
//! "它交回了一枚孔"那件事**不在这里**——归 [`session`](crate::session)：会话的建立与
//! 认领是另一份协议，本文件只剩"起一个服务"需要的那几手。

use env::{EnvError, Name, Permission, PieToken, ProgramKind, TaskId, TeamId};

use runtime::env::mail;
use runtime::env::room;
use runtime::env::unit;

use super::service::Fail;

/// 建域（Mint）：镜像字节 + 特权级 + 名字 → 新域。
///
/// 产出的域归**调用者**（父亲 = 调用者自己）。特权级由**清单**决定、调用方转交
/// ——程序自称不了特权级（这是"放开建域不构成提权"的那一半）。
pub(super) fn mint(image: &[u8], kind: ProgramKind, name: Name) -> Result<TeamId, Fail> {
    unit::build(image, kind, name.as_str()).map_err(fail)
}

/// 产代表线程（未放行）：`entry = 0` ⇒ 走域默认入口。
pub(super) fn bear(team: TeamId) -> Result<TaskId, Fail> {
    unit::spawn(team, 0, &[], 0).map_err(fail)
}

/// 把一枚门闩塞进目标线程手里（放行前做）。**给多大权由调用方定**——这里不替它做主。
pub(super) fn accord(token: PieToken, rep: TaskId, perm: Permission) -> Result<(), Fail> {
    mail::accord(token, rep, perm).map_err(fail)?;
    Ok(())
}

/// 放行。
pub(super) fn hatch(rep: TaskId) -> Result<(), Fail> {
    unit::hatch(rep).map_err(fail)
}

/// 收掉目标所属的域（连它的线程一起）。**Ruin：不靠血缘。**
///
/// 判活照旧：目标不在世 / 从未入册 ⇒ 已经是死的，视作收到（幂等）。
pub(super) fn ruin(rep: TaskId) {
    let _ = room::doom(rep);
}

/// 它收尾完了没有。
pub(super) fn reaped(rep: TaskId) -> Result<bool, Fail> {
    unit::join(rep, 0).map_err(fail)
}

/// 它还活着没有 = "还没收尾完"。
pub(super) fn running(rep: TaskId) -> bool {
    !reaped(rep).unwrap_or(true)
}

/// 等它收尾完：`ms` 三态（`0` 只探、`usize::MAX` 挂到它收尾、其余毫秒）。
/// 返 `true` = **调用开始时**它已经收尾完了。
pub(super) fn until(rep: TaskId, ms: usize) -> Result<bool, Fail> {
    if ms == 0 {
        return reaped(rep);
    }
    if reaped(rep)? {
        return Ok(true);
    }
    // 无限期限：直接挂到它收尾（内核在它回收时唤醒等待者）；其余：睡一段再问一次。
    if ms == usize::MAX {
        let _ = unit::join(rep, usize::MAX);
        return Ok(true);
    }
    let _ = room::sleep(core::time::Duration::from_millis(ms as u64));
    reaped(rep)
}

/// 内核负码 → 本协议的失败域（按"调用方接下来干什么"分，不按内核哪一步坏了）。
fn fail(e: erra::Error<EnvError>) -> Fail {
    match e.source.code() {
        -6 => Fail::BadImage,
        -4 => Fail::NoRoom,
        _ => Fail::Unknown,
    }
}
