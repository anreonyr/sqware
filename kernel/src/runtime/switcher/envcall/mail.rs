//! envcall 数据轴（class 5 `MailCall`）—— 消息穿孔的三个操作。
//!
//! 与权柄轴（`envcall/pie.rs`）的分界：本模块**只搬运载荷**，不调用任何
//! 权柄函数（`accord`/`narrow`/`revoke`/`release`/`vestor`/`snap`）。两轴正交，
//! 判据见 `crates/env/src/fid.rs` 文件头。
//!
//! `ident` 的所有权移交与门面一致：可能触发 halt 的分支先 `drop(ident)`，否则
//! halt 时身份 Arc 仍持最后任务 team → space 不 drop，关机审计会误报帧泄漏。

use alloc::sync::Arc;
use alloc::vec::Vec;

use env::{Fail, HoleDir, MailCall, PieToken, TaskId, Wait};

use riscv::register::sie;

use crate::memory::PAGE_SIZE;
use crate::memory::manager::addr::VirtAddr as KVirt;
use crate::runtime::switcher::context::{Gprs, TrapContext};
use crate::work::mail;
use crate::work::room::messenger::Handoff;
use crate::work::room::scheduler::core::current;
use crate::work::unit::gate::{self, AnyPie, Need};

use super::pie::usable;
use crate::work::unit::task::TaskIdent;

/// 一次 envcall 的落点：续跑本任务，或换一帧跑（让出/挂起）。
///
/// 把「本操作是否可能换帧」写进类型，而不是留给调用方猜。
pub(crate) enum Outcome {
    /// 续跑当前任务：`frame` 即入参帧，返回值已写入 a0/a1。
    Resume,
    /// 换一帧跑：挂起/让出后交出的下一帧。
    Park(*mut TrapContext),
}

/// 数据轴的三个操作。返回 `None` = 本次调用不属于本轴（交还门面继续匹配）。
pub(crate) fn dispatch(
    frame: &mut TrapContext,
    call: MailCall,
    ident: Arc<TaskIdent>,
) -> Option<Outcome> {
    Some(match call {
        MailCall::Push { token, msg, len } => {
            push(frame, ident, token, KVirt::from_raw(msg.get()), len)
        }
        MailCall::Pull { token, buf, max } => {
            pull(frame, ident, token, KVirt::from_raw(buf.get()), max)
        }
        MailCall::Wait { token, dir, millis } => wait_dir(frame, ident, token, dir, millis),
        MailCall::Hush { token } => hush(frame, token),
        MailCall::Ring { token } => ring(frame, token),
    })
}

/// 把一条消息推入 hole：权柄判定（W）→ 长度校验 → 锁外拷入堆暂存 → try_push。
///
/// `me` = 推者身份：内核盖章，与消息同槽交付收方（收方不必从报文里猜发送者）。
fn push(
    frame: &mut TrapContext,
    ident: Arc<TaskIdent>,
    token: PieToken,
    msg: KVirt,
    len: usize,
) -> Outcome {
    let me = current()
        .running_task()
        .map(|t| t.ident.id)
        .unwrap_or(TaskId::new(0));
    // ① 取用判据在核心（`gate::accede`）：表里没有 → `Denied`；已封印 → `Dead`；权不够 →
    //    `Denied`——**顺序只有那一处**，与权柄轴同一个答案。
    //    落点与拷贝两步另有码：`hole::try_push` 答 `Busy`（槽已满）/`Dead`（封印），
    //    锁外暂存的 `try_reserve` 答 `OoM`（**一页以内**备不下），长度不在 `1..=一页`
    //    与区间未映射答 `Denied`。
    //    "被关住"仍住本层：它要核对**别人**的表（L3），故必须在放开本任务 `pies` 之后判。
    let found = current()
        .running_task()
        .ok_or(Fail::Denied)
        .and_then(|t| gate::accede(&t, token, Need::Store));
    let r = match found {
        Err(e) => Err(e),
        // ② 锁外：第四道判据（陈旧锚在此自愈）。
        Ok(pie) => match usable(&pie) {
            Err(e) => Err(e),
            Ok(()) => match &pie {
                AnyPie::Hole(p) => {
                    let meta = p.meta().clone();
                    // 长度校验：**一个区间** `1..=一页`。两头同一个下一步（`Denied`），且都在下面
                    // 那次 `try_reserve` **之前**——界不许先花内存。一页是**载体**的界（不是某一孔
                    // 的），契约在 `env::fid` 的 `Push`。
                    if !(1..=PAGE_SIZE).contains(&len) {
                        Err(Fail::Denied)
                    } else {
                        // 锁外拷入堆暂存：slot = L3，Space.segments = L2，
                        // 持 L3 调 L2 是 4→2 反向嵌套，禁止。
                        // 堆暂存：`try_reserve` 而不是 `vec![0u8; len]`——这一笔不可失败时
                        // 是一次整机 halt，而入口（envcall）本来就能答 `Denied`/`OoM`。
                        // 这份 staging **整个移进槽**（`try_push` 里 swap）：推之后槽里就是它，
                        // 故"消息多长槽就多大"，没有第二份拷贝、没有预留容量。
                        let mut staging: Vec<u8> = Vec::new();
                        if staging.try_reserve(len).is_err() {
                            Err(Fail::OoM)
                        } else {
                            staging.resize(len, 0);
                            if mail::copy_in(&ident.team.space, &mut staging, msg.as_usize()) {
                                mail::hole::try_push(&meta, &mut staging, me)
                            } else {
                                Err(Fail::Denied)
                            }
                        }
                    }
                }
                _ => Err(Fail::Denied),
            },
        },
    };
    frame.gpr.set_x(
        Gprs::A0,
        match r {
            Ok(()) => 0,
            Err(e) => e.code() as usize,
        },
    );
    Outcome::Resume
}

/// 从 hole 取一条消息：权柄判定（R）→ `max == 0` 则**只报长度**，否则把整条消息
/// 移出槽、锁外拷回用户。
///
/// a0 = 实际长度、a1 = 发送者 task id；发送者是内核在 Push 时盖的章，不可伪造。
/// `max` = 收方缓冲容量：装不下（`len > max`）答 `Denied` 且**槽一个字节都不动**；
/// **空槽答 `Busy`**（没有可取之事，`max == 0` 的探长也一样）、**已封印答 `Dead`**
/// （两条都先过 `meta.alive()`）——与 `wait` 的
/// "未就绪答 `false`"是非阻塞/阻塞两面，见 `fid.rs` 的 `Pull` 契约。
fn pull(
    frame: &mut TrapContext,
    ident: Arc<TaskIdent>,
    token: PieToken,
    buf: KVirt,
    max: usize,
) -> Outcome {
    // ① 取用判据在核心（`gate::accede`）；"被关住"在锁外判（见 `push` 的同两段）。
    let found = current()
        .running_task()
        .ok_or(Fail::Denied)
        .and_then(|t| gate::accede(&t, token, Need::Fetch));
    let r = match found {
        Err(e) => Err(e),
        // ② 锁外：第四道判据（陈旧锚在此自愈）。
        Ok(pie) => match usable(&pie) {
            Err(e) => Err(e),
            Ok(()) => match &pie {
                AnyPie::Hole(p) => {
                    let meta = p.meta().clone();
                    if max == 0 {
                        // `max == 0` = **只报长度、不动槽**（与 `Wait { millis: 0 }`「只探测
                        // 不挂起」同一形状的"只问"）：收方据此备出装得下的缓冲。
                        mail::hole::peek(&meta)
                    } else {
                        // 整条消息**移出**槽：零拷贝、锁外无分配（对照旧版按 `max` 预分配
                        // 暂存再拷一遍）。消息的 Vec 就是拷给用户之前的落点。
                        match mail::hole::try_take(&meta, max) {
                            Ok((msg, from)) => {
                                if mail::copy_out(&ident.team.space, &msg, buf.as_usize()) {
                                    Ok((msg.len(), from))
                                } else {
                                    Err(Fail::Denied)
                                }
                            }
                            Err(e) => Err(e),
                        }
                    }
                }
                _ => Err(Fail::Denied),
            },
        },
    };
    // 正路径：a0 = 实际长度、a1 = 发送者 task id；错误路径 a0 = 负码。
    match r {
        Ok((n, from)) => {
            frame.gpr.set_x(Gprs::A0, n);
            frame.gpr.set_x(Gprs::A1, from.get());
        }
        Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
    }
    Outcome::Resume
}

/// 等就绪：权柄判定（`dir` 决定 R 还是 W）→ 探测或挂起。
///
/// `millis` 是上限族（`Wait::Forever` = 永久、`Wait::POLL` = 只探测不挂起；
/// 过线那一格拼成 `usize::MAX` / `0`）。a0 返 `true` = 本次调用
/// **当场就绪**（未挂起）；`false` = 未就绪（探测失败，或被唤醒/超时——两者不分）。
/// **绝不返 `-3 Busy`**：未就绪的答案就是 `false`。
///
/// **两条资源通道**：孔有方向（`dir` 要 R 或 W），铃只有一条——故 Nole 只认
/// `dir == Pull`，别的值返 `Denied`。
fn wait_dir(
    frame: &mut TrapContext,
    ident: Arc<TaskIdent>,
    token: PieToken,
    dir: HoleDir,
    millis: Wait,
) -> Outcome {
    /// 等的是哪一条通道：孔指向具体方向，铃就是铃。
    enum Ready {
        Hole(Arc<mail::hole::HoleMeta>),
        Bell(Arc<mail::nole::NoleMeta>),
    }
    // ① 锁内解析 token ⇒ 抄件：pies 与站点表同为 L3，绝不嵌套；
    //    `running_task` 的临时强引用在闭包内即 drop，不跨挂起。
    let need = match dir {
        HoleDir::Pull => Need::Fetch,
        HoleDir::Push => Need::Store,
    };
    let found = current()
        .running_task()
        .ok_or(Fail::Denied)
        .and_then(|t| gate::accede(&t, token, need));
    // ② 锁外：第四道判据（陈旧锚在此自愈）。
    let resolved = match found {
        Err(e) => Err(e),
        Ok(pie) => match usable(&pie) {
            Err(e) => Err(e),
            Ok(()) => match &pie {
                AnyPie::Hole(p) => Ok(Ready::Hole(p.meta().clone())),
                // 铃只有"响了"一条方向：别的方向不是"暂时没有"，是不存在这个操作。
                AnyPie::Nole(p) if dir == HoleDir::Pull => Ok(Ready::Bell(p.meta().clone())),
                _ => Err(Fail::Denied),
            },
        },
    };
    let dur = millis.into_duration();
    match resolved {
        Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
        Ok(ready) => {
            // 挂起路径的默认返回 = false（未当场就绪）；可能 halt 的分支先放身份。
            frame.gpr.set_x(Gprs::A0, 0);
            drop(ident);
            // 两条通道的原语**同型**（`Handoff<bool>`），故这里只挑调用、不各写一遍答案：
            // `Resume(true)` = 该方向/铃**当场就绪**，`Resume(false)` = 未就绪且 `millis == 0`。
            let parked = match &ready {
                Ready::Hole(meta) => mail::hole::wait(meta, dir, dur),
                Ready::Bell(meta) => mail::nole::wait(meta, dur),
            };
            match parked {
                Ok(Handoff::Resume(ready)) => frame.gpr.set_x(Gprs::A0, ready as usize),
                Ok(Handoff::Switch(pa)) => return Outcome::Park(pa as *mut TrapContext),
                Err(e) => frame.gpr.set_x(Gprs::A0, e.code() as usize),
            }
        }
    }
    Outcome::Resume
}

/// 应铃：权柄判定（R）→ 清掉"有待取之事"，并**重开本 hart 的外部中断闸门**。
///
/// 闸门那一半与 `devices::raise_irq` 的 `Err ⇒ clear_sext` 配对：内核在"响还在"时
/// 关闸门，用户在这里说"我取走了"。闸门本身是**零状态**的（`trap/mod.rs` 的 timer tick
/// 无条件重开），故这一句只是让重开**立即**发生，而不是让它成为唯一路径。
///
/// 不收 `ident`：本操作不挂起、不 halt（可能 halt 的分支才需要先放身份）。
fn hush(frame: &mut TrapContext, token: PieToken) -> Outcome {
    let r = with_bell(token, Need::Fetch, mail::nole::hush);
    if r.is_ok() {
        // SAFETY: 与 trap 分支那一句同源：只置本 hart 的 SEIE 位。
        unsafe {
            sie::set_sext();
        }
    }
    frame.gpr.set_x(
        Gprs::A0,
        match r {
            Ok(()) => 0,
            Err(e) => e.code() as usize,
        },
    );
    Outcome::Resume
}

/// 响铃：权柄判定（W）→ 置"有待取之事"并唤醒听者。不搬任何字节。
fn ring(frame: &mut TrapContext, token: PieToken) -> Outcome {
    let r = with_bell(token, Need::Store, mail::nole::ring);
    frame.gpr.set_x(
        Gprs::A0,
        match r {
            Ok(()) => 0,
            Err(e) => e.code() as usize,
        },
    );
    Outcome::Resume
}

/// 应铃／响铃共用的三段：**锁内**定位 + 判权 + 判活 ⇒ 抄件；**锁外**判"被关住"，
/// 再落那一个动作。
///
/// 与 `push`/`pull` 的 ①② 同构（那两个各自内联了一遍）；两者只差 `need` 与落在
/// meta 上的动作，故收 `op` 而不是抄两遍。
fn with_bell(
    token: PieToken,
    need: Need,
    op: fn(&mail::nole::NoleMeta) -> Result<(), Fail>,
) -> Result<(), Fail> {
    let found = current()
        .running_task()
        .ok_or(Fail::Denied)
        .and_then(|t| gate::accede(&t, token, need));
    match found {
        Err(e) => Err(e),
        // ② 锁外：第四道判据（陈旧锚在此自愈）。
        Ok(pie) => match usable(&pie) {
            Err(e) => Err(e),
            Ok(()) => match &pie {
                AnyPie::Nole(p) => op(p.meta()),
                _ => Err(Fail::Denied),
            },
        },
    }
}
