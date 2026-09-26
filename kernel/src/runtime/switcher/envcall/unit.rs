// Unit 域（class 1）—— 单元：产 / 认 / 装 / 放 / 收 / 等 / 夺。
//
// 本域的本体只住本文件：两件**折算**（`map_err` 折 `MapError`、`copy_words` 读调用方
// 空间）也住这里——它们的唯一读者是 `Spawn`。

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;

use env::{TaskId, UnitCall, UnitFail};

use crate::memory::manager::MapError;
use crate::memory::manager::addr::VirtAddr as KVirt;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::room::messenger::{self, Handoff};
use crate::work::room::scheduler::core::{current, muster};
use crate::work::unit::life::TaskLife;
use crate::work::unit::source::Source;
use crate::work::unit::space::{Space, SpaceKind};
use crate::work::unit::task::{MAX_ARGS, Task, TaskIdent, TaskTag};
use crate::work::unit::team::UnitError;
use crate::work::unit::weak::{Site, TaskWeak};

use super::ret_err;

/// 一次单元 envcall 的落点：续跑本帧，或换一帧跑。
pub(super) enum Outcome {
    /// 续跑当前任务：`frame` 即入参帧，返回值已写入 a0。
    Resume,
    /// 换一帧跑：`Join` / `Fall` 挂起后交出的下一帧。
    Switch(*mut TrapContext),
}

impl Outcome {
    /// 当场答负码、未离核：本域的失败收尾只有这一种。
    fn fail<E: env::FailCode>(frame: &mut TrapContext, e: E) -> Self {
        ret_err(frame, e);
        Self::Resume
    }
}

/// 映射错误 → 本域词汇（`Spawn` 的栈/帧分配失败）。
fn map_err(e: MapError) -> UnitFail {
    match e {
        MapError::OutOfMemory => UnitFail::OoM,
        // 其余（对齐 / 已映射 / 未映射 / 无区段 / 借入加宽 / 段状态不符 / 恒等压栈）在
        // "产线程的栈与帧"这条路上都是"要的东西给不出" ⇒ `Denied`。**穷尽 match**：
        // `MapError` 多一枚变体就编不过（同一个内核错误在 Memory 域另有一处折算）。
        MapError::NotAligned
        | MapError::AlreadyMapped
        | MapError::NotMapped
        | MapError::NoRegion
        | MapError::WidenDenied
        | MapError::SegmentMismatch
        | MapError::DramOverlap => UnitFail::Denied,
    }
}

/// 读调用方空间里的 `count` 个字（`Spawn` 的启动参数）。
///
/// 缓冲**定长在栈上**（`MAX_ARGS · 8` = 512 B）：`count` 的界就是 `MAX_ARGS`，不必为
/// 它分配。源与 `Build` 同一件——见 [`Source`]。
fn copy_words(space: &Space, va: KVirt, count: usize) -> Option<Vec<usize>> {
    if count > MAX_ARGS {
        return None;
    }
    let width = size_of::<usize>();
    let len = count * width;
    let mut bytes = [0u8; MAX_ARGS * size_of::<usize>()];
    let src = Source::Space { space, va, len };
    if !src.read(0, &mut bytes[..len]) {
        return None;
    }
    // 同 `Source::read` 那条：**可失败，不 panic**——`Vec::with_capacity` 走
    // `handle_alloc_error`（内存吃紧 ⇒ 整机 halt），失败与"区间未映射"同路返回
    // `None`，调用方把两类都落到 `Denied` 上，机器照旧活着。
    let mut out: Vec<usize> = Vec::new();
    out.try_reserve(count).ok()?;
    for i in 0..count {
        let mut w = [0u8; size_of::<usize>()];
        w.copy_from_slice(&bytes[i * width..(i + 1) * width]);
        out.push(usize::from_le_bytes(w));
    }
    Some(out)
}

/// 本域的臂。
pub(super) fn dispatch(frame: &mut TrapContext, call: UnitCall, ident: Arc<TaskIdent>) -> Outcome {
    match call {
        UnitCall::Spawn {
            team,
            entry,
            args,
            count,
            stack,
        } => {
            // 目标域：TeamId(0) = 当前域；否则必须在我 heir 里（查到 = 我是 sire）
            let target = if team.get() == 0 {
                ident.team.clone()
            } else {
                match current().running_task().and_then(|me| me.heir(team)) {
                    Some(t) => t,
                    None => return Outcome::fail(frame, UnitFail::Denied),
                }
            };
            // 启动参数：从调用方空间拷（count == 0 → 空）
            let words = match copy_words(&ident.team.space, KVirt::from_raw(args.get()), count) {
                Some(w) => w,
                None => return Outcome::fail(frame, UnitFail::Denied),
            };
            // entry = 0 → 域默认入口（`Build` 装载所得 e_entry）
            let entry_va = if entry == 0 {
                target.default_entry()
            } else {
                entry
            };
            let mut builder = target.task().entry(KVirt::from_raw(entry_va)).args(words);
            if stack > 0 {
                builder = builder.stack(stack);
            }
            // 恒产 Held：授权顺序由父方 `Accord` → `Hatch` 保证
            match builder.hold() {
                Ok(t) => frame.gpr.set_x(Gprs::A0, t.ident.id.get()),
                Err(e) => return Outcome::fail(frame, map_err(e)),
            }
            Outcome::Resume
        }
        UnitCall::SelfId => {
            let id = current()
                .running_task()
                .map(|t| t.ident.id)
                .unwrap_or(TaskId::new(0));
            frame.gpr.set_x(Gprs::A0, id.get());
            Outcome::Resume
        }
        UnitCall::Sire => {
            // 溯源：生我者的 task id。0 = 顶级域（boot）或父已亡。
            let id = current()
                .running_task()
                .and_then(|t| t.ident.team.sire())
                .unwrap_or(TaskId::new(0));
            frame.gpr.set_x(Gprs::A0, id.get());
            Outcome::Resume
        }
        UnitCall::HeirCount => {
            // 我生的子域数量（heir 枚举 first pass）。
            let n = current()
                .running_task()
                .map(|t| t.heir_count())
                .unwrap_or(0);
            frame.gpr.set_x(Gprs::A0, n);
            Outcome::Resume
        }
        UnitCall::Heir { index } => {
            // 按索引取子域 TeamId（heir 枚举 second pass；越界 → 0）。
            let id = current()
                .running_task()
                .and_then(|t| t.heir_at(index))
                .map(|t| t.get())
                .unwrap_or(0);
            frame.gpr.set_x(Gprs::A0, id);
            Outcome::Resume
        }
        UnitCall::Build { elf, len, kind } => {
            // 门：**没有门**（Mint 口径）。
            //
            // 曾经这里是"建域权就是 S 态"——那是**内核替调用方定"该不该起"的时候**
            // 留下的。判据现在已经分家：能不能起由本层回答（答"能"），该不该起归
            // `protocol::system` 的编排者（它拿服务表与策略说话）。
            //
            // 放开它**不构成提权**：提权的两条路都还堵着——特权级由内核打包表决定
            // （调用方说不上话），镜像仍要调用方交字节（Mint-lite 口径，见该协议正文）。
            //
            // 镜像：**不拷**。`Source` 把"这份字节从哪来"交给 loader 逐段现取——一份
            // ELF 里 97% 是符号表与调试信息（实测 18 台 debug 镜像：78.2 MiB 文件、
            // 2.13 MiB 段实体），从前先整份搬进内核暂存再丢掉，是白搬的那一跳。
            let source = Source::Space {
                space: &ident.team.space,
                va: KVirt::from_raw(elf.get()),
                len,
            };
            // sire = 调用方：`build` 内部闭合血缘（域必入我 heir）。
            //
            // 这枚弱引用的出身是**血亲**（`Site::Sire`）：它几步之后就会住进
            // `Team.sire`，中间没有挂起点（`build` 全程不 switch）。写清出身是为了
            // 弱引用收支账能把这枚与"抄件"分开（见 `work::unit::weak`）。
            let sire = match current().running_task() {
                Some(me) => TaskWeak::stored(Arc::downgrade(&me), Site::Sire),
                None => TaskWeak::empty(),
            };
            match crate::work::unit::build(&source, SpaceKind::from(kind), sire) {
                Ok(team) => frame.gpr.set_x(Gprs::A0, team.id.get()),
                // 源读不到 = 调用方自己的映射不在（或本域另一枚线程刚放手）——与从前
                // "暂存拷不进来"同一个负码。
                Err(UnitError::Unreadable) => return Outcome::fail(frame, UnitFail::Denied),
                // 内存不够从"镜像不认"里分出来：`OoM` 这一格编排者本来就接
                // （`protocol::system::core::Fail::Full`），`BadImage` 没有。
                Err(UnitError::OoM) => return Outcome::fail(frame, UnitFail::OoM),
                Err(UnitError::Load) => return Outcome::fail(frame, UnitFail::BadImage),
            }
            Outcome::Resume
        }
        UnitCall::Hatch { task } => {
            let target = match muster(task).and_then(|w| w.upgrade()) {
                Some(t) => t,
                None => return Outcome::fail(frame, UnitFail::Denied),
            };
            // 授权：与我同域，或属于我 heir 里的子域
            let same = Arc::ptr_eq(&target.ident.team, &ident.team);
            let mine = current()
                .running_task()
                .map(|me| me.heir(target.ident.team.id).is_some())
                .unwrap_or(false);
            if !(same || mine) {
                return Outcome::fail(frame, UnitFail::Denied);
            }
            if let Err(e) = Task::release(&target) {
                return Outcome::fail(frame, e);
            }
            Outcome::Resume
        }
        UnitCall::Join { task, millis } => {
            let dur = millis.into_duration();
            // 判活三态**在边界一次问清**（room 不查注册表）：
            //   ① 名册里没有这个 id ⇒ **从未分配** = 非法 id ⇒ Denied。旧版把这一支与
            //      「目标仍活」折在一起（判活有两个真相源时必然如此），于是非法 id 拿到
            //      「未回收」、`Join{0}` 拿到「已回收」；而那个本该拦它的 `Err(Denied)`
            //      需要 `target_dead ∧ ¬allocated` 同时成立，两条来路都蕴含 `allocated`
            //      ⇒ 它**曾经永远不可达**。
            //   ② 升不起强引用 ⇒ 已消失（对象已回收）⇒ 当场结论「已回收」；授权无从核对
            //      （照旧放行；寿命无从谈起 ⇒ 空弱引用，站点当场判死、不建站点）。
            //   ③ 仍是活任务 ⇒ 当场核对授权，并把「退出钩子是否已跑完」读出来。
            let Some(target) = muster(task) else {
                return Outcome::fail(frame, UnitFail::Denied);
            };
            // **挂起前放掉那枚抄件**（`muster` 抄出来的弱引用）：`target` 只用来当场判活
            // 与取 `(reaped, life)`，此后它就是一具"跨挂起还压在栈上"的引用 —— 而
            // `messenger::join` 会挂起本任务。这条链一旦被别核判死（或被收尾就地冻住），
            // `target` 的 `Drop` 永不执行：目标任务的 `ArcInner` 外壳（152 B）被一枚
            // **永远活着**的弱引用扣住 ⇒ 关机审计 `leak: task 1`（`strong 0 weak 1`）。
            // 与下面那行 `drop(ident)` 是同一条纪律（见 `dispatch` 头注）；
            // 弱引用同样算"引用"，只是它钉住的是外壳而不是载荷。
            let (reaped, life) = match target.upgrade() {
                Some(t) => {
                    let same = Arc::ptr_eq(&t.ident.team, &ident.team);
                    let mine = current()
                        .running_task()
                        .map(|me| me.heir(t.ident.team.id).is_some())
                        .unwrap_or(false);
                    if !(same || mine) {
                        return Outcome::fail(frame, UnitFail::Denied);
                    }
                    (t.tag() == TaskTag::Reaped, t.life())
                }
                None => (true, Weak::new()),
            };
            // 挂起后恢复读到的 a0 = 挂起前预置值 ⇒ 预置 0（未回收）；当场判定再改写
            frame.gpr.set_x(Gprs::A0, 0);
            drop(ident);
            drop(target);
            match messenger::join::<UnitFail>(TaskLife { id: task, life }, reaped, dur) {
                // 未离核：当场结论（true = 调用开始时目标已回收）。
                Ok(Handoff::Resume(dead)) => {
                    frame.gpr.set_x(Gprs::A0, dead as usize);
                    Outcome::Resume
                }
                Ok(Handoff::Switch(pa)) => Outcome::Switch(pa as *mut TrapContext),
                // 备料失败（内存耗尽）：本任务**没挂起**，当场答 `OoM`。
                Err(e) => Outcome::fail(frame, e),
            }
        }
        UnitCall::Fall { millis } => {
            let dur = millis.into_duration();
            // **只等"我自己这张表"**：键由内核从调用者推出来，故这里没有参数、
            // 也就没有伪造面（同 `SelfId` / `Sire` 那一路）。
            let Some(me) = current().running_task() else {
                return Outcome::fail(frame, UnitFail::Busy);
            };
            let mine = TaskLife {
                id: me.ident.id,
                life: me.life(),
            };
            // 挂起后恢复读到的 a0 = 挂起前预置值 ⇒ 预置 0（没落过）；当场判定再改写。
            frame.gpr.set_x(Gprs::A0, 0);
            drop(ident);
            // 跨挂起不得持强引用（同 `Join` 那条纪律）：只留那份弱引用。
            drop(me);
            match messenger::fall::<UnitFail>(mine, dur) {
                Ok(Handoff::Resume(landed)) => {
                    frame.gpr.set_x(Gprs::A0, landed as usize);
                    Outcome::Resume
                }
                Ok(Handoff::Switch(pa)) => Outcome::Switch(pa as *mut TrapContext),
                Err(e) => Outcome::fail(frame, e),
            }
        }
        UnitCall::Oust { team } => {
            let Some(me) = current().running_task() else {
                return Outcome::fail(frame, UnitFail::Denied);
            };
            // 凭证就是**我自己那张血缘表**（同 `Spawn` 的门）：查到 = 我是它的 sire。
            let Some(child) = me.heir(team) else {
                return Outcome::fail(frame, UnitFail::Denied);
            };
            // 前置：域里没有还没收尾的线程（判据读法与"回收对调用方不可观测"那条一致）。
            if !child.all_reaped() {
                return Outcome::fail(frame, UnitFail::Busy);
            }
            // 手里那份瞬时引用先还掉：摘除只需 id，析构留给锁外。
            drop(child);
            // **摘除与析构分开**：`oust` 在表锁内只做 Vec 摘除，交回的那一份在这里落地
            // ⇒ `Team`（连带 `Space`）的析构不在 L3 锁里走。
            drop(me.oust(team));
            Outcome::Resume
        }
    }
}
