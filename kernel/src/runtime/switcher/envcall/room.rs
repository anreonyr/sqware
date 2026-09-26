// Room 域（class 0）—— 会话：饿 / 退 / 杀 / 挂 / 等 / 醒。
//
// 本域的本体只住本文件。
//
// **本域的落点是三态**（见 [`Outcome`]）：七格有三种收尾——续跑本帧、换一帧、
// 本任务退场。第三种只有 `Reap` 有，而它是"退场窄尾"的标记（空指针）：写成枚举变体，
// 故"既换帧又退场"这种状态不可表达。

use core::time::Duration;

use alloc::sync::Arc;

use env::{RoomCall, RoomFail};

use crate::runtime::diagnose::trace::{self, EventKind, RoomEvent};
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::room::messenger::{self, Handoff, WakeKey, park, park_until, wait, wake};
use crate::work::room::scheduler::core::{current, muster};
use crate::work::unit::task::TaskIdent;

use super::ret_err;

/// 一次会话 envcall 的落点：续跑本帧、换一帧跑，或本任务退场。
pub(super) enum Outcome {
    /// 续跑当前任务：`frame` 即入参帧，返回值已写入 a0。
    Resume,
    /// 换一帧跑：让出/挂起后交出的下一帧。
    Switch(*mut TrapContext),
    /// 本任务退场：`dispatch` 收这一态时归还**空指针**，由最浅的 Rust 帧收尾。
    Exit,
}

impl Outcome {
    /// 当场答负码、未离核：本域的失败收尾只有这一种。
    fn fail<E: env::FailCode>(frame: &mut TrapContext, e: E) -> Self {
        ret_err(frame, e);
        Self::Resume
    }
}

/// 本域的臂。
pub(super) fn dispatch(frame: &mut TrapContext, call: RoomCall, ident: Arc<TaskIdent>) -> Outcome {
    match call {
        RoomCall::Starve => Outcome::Switch(current().starve() as *mut TrapContext),
        RoomCall::Reap { reason, note, len } => {
            // 本任务退场：**不在这里 quit**（退场窄尾：最浅的 Rust 帧收尾）——空指针即标记。
            //
            // `reason` 是**数据**：0 = 自愿/正常结束，非 0 = 域自己的诊断编号。内核只把
            // 它记进 trace，**不解释**——"域为什么不可续"是域的判断，内核的事只是
            // "它不再续跑"与"把它的账结清"。故本仓**没有** `ControlCall::Panic`
            // 这样的第二入口：那会把域的策略写进 ABI，并让"任务终止"
            // 这条不变量在 ABI 里有两个出口。
            //
            // 写进逐核暂存槽，由 `quit` 统一发出 `RoomEvent::Exit`：那是**所有**退出
            // 路径（Reap / 故障隔离 / doom 级联）的公共点，事件因此只发一次、
            // 且每条路径都带得上原因（故障路径带走的是内核给的原因码）。
            //
            // `note` = 域自己带的一句话（`len = 0` = 无话）：这里只落下它的**位置**——
            // 打印与入账都在 `quit`（退场路径的公共点），而读它的时点仍在
            // `reap`/`bury` 之前，那段空间还在（见 `messenger::EXIT_NOTE`）。
            crate::work::room::messenger::set_exit_note(note.get(), len);
            crate::work::room::messenger::set_exit_reason(reason);
            drop(ident);
            Outcome::Exit
        }
        RoomCall::Doom { task } => {
            // 他杀（与 `Reap` 成对：自杀 ↔ 他杀）。判据只有**判活**，**没有血缘门**
            // ——这是 `doom` 口径：收一个域是"命令"，不是"血缘特权"。
            //
            // 曾经这里要 `descends`（目标域得在我后代链里）。删掉它的理由是判据分家：
            // 内核只回答"能不能收"（能），"该不该收"归 `protocol::system` 的编排者
            // （它拿服务表说话）。与建域那一支是同一次分家，见 `UnitCall::Build`。
            //
            // 代价照实记：**服务之间因此没有护栏**（任何域都能拆任何域）。收窄只能在
            // 编排侧做（"谁能申请收谁"），不是内核该长的东西。
            //
            // 语义仍是**域粒度**：`task` 只是"指认域"的手柄，它所属的域连同子树一起走
            // （同域的线程一并，不会剩半个域）——执行复用结构面既有的两相扑杀。
            //
            // 一次调用**只下一道令**，不下场等它回收：要等就 `UnitCall::Join`
            // （Linux 的 `kill` 也是"送到即回"）。
            let target = muster(task).and_then(|w| w.upgrade());
            let Some(target) = target else {
                // 名册升不起来 = 从未入册 / 已回收——与 `Join` 判活三态同一口径。
                return Outcome::fail(frame, RoomFail::Dead);
            };
            let team = target.ident.team.clone();
            // **判活是域粒度**：域里已没有还没收尾的线程 ⇒ 与"名册升不起"同答 `Dead`，
            // 不再"答成功却什么都没做"（读法与 `Team::all_reaped` 同一句）。
            // 空域够不到这一支——它没有 `TaskId` 手柄，那条边界照旧（见 `protocol::system` §八）。
            if team.all_reaped() {
                return Outcome::fail(frame, RoomFail::Dead);
            }
            // 下令时记一笔（谁杀的）；死亡时受害者那颗核另记 `Exit { EXIT_DOOM }`
            // ——两条分开是因为它们落在不同的核上（见 `RoomEvent::Doomed`）。
            trace::note(EventKind::Room(RoomEvent::Doomed {
                tid: target.ident.id.get(),
                by: ident.id.get(),
            }));
            drop(target);
            drop(ident);
            messenger::cull(&[team], messenger::EXIT_DOOM);
            frame.gpr.set_x(Gprs::A0, 0);
            Outcome::Resume
        }
        RoomCall::Park { millis } => {
            drop(ident);
            match park::<RoomFail>(Duration::from_millis(millis as u64)) {
                Ok(pa) => Outcome::Switch(pa as *mut TrapContext),
                // 备料失败（内存耗尽）：本任务**没挂起**，当场答 `OoM`。
                Err(e) => Outcome::fail(frame, e),
            }
        }
        RoomCall::ParkUntil { at } => {
            drop(ident);
            match park_until::<RoomFail>(at) {
                // 到点已过 ⇒ 未离核即续跑（ABI 契约：当场返回，不是让出一拍）。
                Ok(None) => Outcome::Resume,
                Ok(Some(pa)) => Outcome::Switch(pa as *mut TrapContext),
                // 备料失败（内存耗尽）：本任务**没挂起**，当场答 `OoM`。
                Err(e) => Outcome::fail(frame, e),
            }
        }
        RoomCall::Wait { key, millis } => {
            // 键 → 存活单元：**解析在调用方这一层**（room 不认识注册表）。空间键的
            // 寿命就是本任务所属空间的寿命，故弱引用随键一起交给等待机。
            let (space, wlife) = {
                let s = &ident.team.space;
                (s.asid(), s.life())
            };
            let wkey = WakeKey::Space { space, slot: key };
            let dur = millis.into_duration();
            drop(ident);
            // `wlife` **按值**交给等待机（站点是它唯一的持有者）。
            match wait::<RoomFail>(wkey, wlife, dur) {
                // `RoomCall::Wait` 没有当场结论：未离核即续跑。
                Ok(Handoff::Resume(())) => Outcome::Resume,
                Ok(Handoff::Switch(pa)) => Outcome::Switch(pa as *mut TrapContext),
                // 备料失败（内存耗尽）：本任务**没挂起**，当场答 `OoM`。
                Err(e) => Outcome::fail(frame, e),
            }
        }
        RoomCall::Wake { key } => {
            let (space, wlife) = {
                let s = &ident.team.space;
                (s.asid(), s.life())
            };
            let wkey = WakeKey::Space { space, slot: key };
            let woke = wake(wkey, &wlife);
            frame.gpr.set_x(Gprs::A0, woke as usize);
            Outcome::Resume
        }
    }
}
