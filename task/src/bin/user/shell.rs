#![no_std]
#![no_main]

//! shell — 命令解释器，经 Terminal（term 模块）显示、读命令并分发给系统能力。
//!
//! 分层：本 bin 是 Shell；`task::term::{Terminal, Readline}` 是渲壳 + 行编辑宿主。
//! **Terminal 是唯一 console 出口**——Shell 的一切输出经 `Terminal::write`/`writeline`、
//! 一切输入经 `Terminal::readline`，不再直接 `io::put`。
//!
//! 命令（系统能力巡演）：
//!   help  — 列命令
//!   clock — 读全局时钟（ChronoCall::Clock）
//!   ticks — 读 timebase 刻度
//!   alloc — 堆分配一页（MemoryCall::Allocate）
//!   echo  — 回显参数
//!   sleep — 阻塞 N 毫秒（RoomCall::Park）
//!   spawn — 派一个算 0..N 的闭包子任务并 join（UnitCall::Spawn + Join）
//!   heir  — 子域枚举（UnitCall::HeirCount + Heir）
//!   hole  — Hole 通道自测（unseal/push/pull/seal）
//!   cascade — 派生级联自检（三跳撤销 / 无关分支 / release 级联 / 任务消亡级联）
//!   reclaim — 资源寿命自检（引用回收 / 封印归属 / 开辟者消亡）
//!   spoof — 身份伪造自检（发送者由内核盖章，报文里的回信 token 不构成身份）
//!   name  — 名字权限自检（目录的名字空间由父域预约，注册只能填预约行）
//!   badslot — 非法 envcall 槽位自检（未知调用号 → 拒掉并续跑，绝不 panic）
//!   req   — 走目录协议连接 echo 并调用一次（Connect + Service::call）
//!   dir   — 目录协议自省（Discover + Enumerate）
//!   exit  — 退出 shell（RoomCall::Reap）

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use core::sync::atomic::{AtomicUsize, Ordering};
use core::time::Duration;

use env::dispatch::{MSG_LEN, Name, Reply, Request};

use task::core::handshake::{self, Pier, Quay};
use task::core::service::{Directory, E_DENIED, E_NOT_FOUND, PAYLOAD_LEN};
use task::core::unit;
use task::env::{
    chrono::{self, clock},
    mail::{self, HOLE_MTU_MAX, HolePie, PolePie},
    room::{self, sleep},
    task::{heir_at, heir_count, join as task_join},
};
use task::term::{Color, Readline, Terminal};

/// 目录请求门闩在**本任务侧**的句柄（启动期握手拿到，此后只读）。
static DIR_ENTRY: AtomicUsize = AtomicUsize::new(0);

/// 启动期握手：靠泊 → 自建控制孔并交给父域 → 报到 → 收配给（目录门闩由 dir 亲授）。
/// 返目录请求门闩在**本任务侧**的句柄。
fn shake() -> env::EnvResult<usize> {
    let up = handshake::moor()?;
    let down = HolePie::unseal(handshake::MTU)?;
    let sire = task::env::task::sire()?;
    let at_parent = down.accord(sire.get(), env::Permission::READ | env::Permission::WRITE)?;
    Quay::new(at_parent).push(&up)?;
    Ok(Pier::pull(&down)?.token())
}

/// 打开一次目录会话（每次新建 reply hole；entry 只是重建句柄）。
fn dir_session() -> env::EnvResult<Directory> {
    Directory::open(HolePie::from_token(DIR_ENTRY.load(Ordering::Relaxed)))
}

/// 按空白切词（保留空输入 = 空 Vec）。
fn split(line: &str) -> Vec<String> {
    line.split_whitespace().map(str::to_string).collect()
}

/// 等目标结束。`Join` 返回真 ⇒ **收尾已完成**（内核契约：退出钩子已跑完），
/// 故调用方拿到真之后即可断言「它名下的门闩与通道都消失了」，无需重试。
///
/// 调用模式与 `MailCall::Wait` 同源：挂起过的那次只当「醒了一次」，须复探。
fn join_done(tid: env::TaskId) {
    loop {
        if task_join(tid, 0).unwrap_or(true) {
            return;
        }
        let _ = task_join(tid, usize::MAX);
    }
}

/// 派生级联自检（`cascade` 命令）。
///
/// 四段判据：
///   1. 三跳 A→B→C：撤销中间那跳 B，末端 C 必须失效；
///   2. 无关分支 D 不受波及，资源本体 A 仍可用；
///   3. `release` 同样级联：放下 A → D 随之下线；
///   4. 任务消亡级联：closure 授出的 Q 随它消亡而失效（退出钩子 `gate::doom`）
///      ——`Join` 返回真即已收尾，故当场断言、不重试。
///
/// 门闩是 per-task 的，同域两个线程也不能共享——故与 closure 的交接一律走
/// 共享内存槽 + `room` 键（与启动期握手同一手法）。
fn cascade(term: &Terminal) {
    const WAIT: usize = 5_000;

    let rw = env::Permission::READ | env::Permission::WRITE;
    let msg = [0x5au8; 8];
    let mut buf = [0u8; 8];

    let me = match unit::self_id() {
        Ok(id) => id,
        Err(_) => {
            term.writeline("cascade: no self id");
            return;
        }
    };
    let a = match HolePie::unseal(HOLE_MTU_MAX) {
        Ok(p) => p,
        Err(_) => {
            term.writeline("cascade: unseal failed");
            return;
        }
    };
    let b = match a.accord(me, rw | env::Permission::VEST) {
        Ok(t) => HolePie::from_token(t),
        Err(_) => {
            term.writeline("cascade: accord b failed");
            return;
        }
    };
    let d = match a.accord(me, rw) {
        Ok(t) => HolePie::from_token(t),
        Err(_) => {
            term.writeline("cascade: accord d failed");
            return;
        }
    };

    // ── 1+2：三跳撤销 + 无关分支 ──
    let c_slot: &'static [AtomicUsize; 1] = Box::leak(Box::new([AtomicUsize::new(0)]));
    let c_ptr = c_slot.as_ptr() as usize;
    let key_c = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let key_ack = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let key_rev = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;

    let join_c = unit::closure(move || {
        let _ = room::wait(key_c, WAIT);
        let c = HolePie::from_token(unsafe {
            (*(c_ptr as *const AtomicUsize)).load(Ordering::Relaxed)
        });
        let mut buf = [0u8; 8];
        // 撤销前：C 可用（B 还在）。
        let before = c.push(&msg).is_ok();
        let _ = c.pull(&mut buf); // 清槽：让 after 的失败只可能来自撤销
        let _ = room::wake(key_ack);
        // 等 shell 撤销 B。
        let _ = room::wait(key_rev, WAIT);
        let after = c.push(&msg).is_ok();
        (before, after)
    });
    let c = match b.accord(join_c.id(), rw) {
        Ok(t) => HolePie::from_token(t),
        Err(_) => {
            drop(join_c);
            term.writeline("cascade: accord c failed");
            return;
        }
    };
    c_slot[0].store(c.token(), Ordering::Relaxed);
    let _ = room::wake(key_c);
    let _ = room::wait(key_ack, WAIT);

    // 撤销 B：C 应随之失效（级联）。
    let revoked = b.revoke(me, b.token()).is_ok();
    let _ = room::wake(key_rev);
    let (before, after) = join_c.join();

    let b_dead = b.push(&msg).is_err();
    let d_ok = d.push(&msg).is_ok();
    let _ = d.pull(&mut buf);
    let a_ok = a.push(&msg).is_ok();
    let _ = a.pull(&mut buf);
    term.writeline(&format!(
        "cascade: revoke={revoked} C.before={before} C.after={after} B.dead={b_dead} D={d_ok} A={a_ok}"
    ));

    // ── 3：release 级联 ──
    let released = a.release().is_ok();
    let d_dead = d.push(&msg).is_err();
    term.writeline(&format!("cascade: release={released} D.after={d_dead}"));

    // ── 4：任务消亡级联 ──
    let r = match HolePie::unseal(HOLE_MTU_MAX) {
        Ok(p) => p,
        Err(_) => {
            term.writeline("cascade: unseal r failed");
            return;
        }
    };
    let r2_slot: &'static [AtomicUsize; 1] = Box::leak(Box::new([AtomicUsize::new(0)]));
    let q_slot: &'static [AtomicUsize; 1] = Box::leak(Box::new([AtomicUsize::new(0)]));
    let r2_ptr = r2_slot.as_ptr() as usize;
    let q_ptr = q_slot.as_ptr() as usize;
    let key_r = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let key_q = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;

    let join_e = unit::closure(move || {
        let _ = room::wait(key_r, WAIT);
        let r2 = HolePie::from_token(unsafe {
            (*(r2_ptr as *const AtomicUsize)).load(Ordering::Relaxed)
        });
        if let Ok(q) = r2.accord(me, rw) {
            unsafe { (*(q_ptr as *const AtomicUsize)).store(q, Ordering::Relaxed) };
        }
        let _ = room::wake(key_q);
    });
    let e_tid = join_e.id();
    let r2 = match r.accord(e_tid, rw | env::Permission::VEST) {
        Ok(t) => HolePie::from_token(t),
        Err(_) => {
            drop(join_e);
            term.writeline("cascade: accord r2 failed");
            return;
        }
    };
    r2_slot[0].store(r2.token(), Ordering::Relaxed);
    let _ = room::wake(key_r);
    let _ = room::wait(key_q, WAIT);
    let q = HolePie::from_token(q_slot[0].load(Ordering::Relaxed));
    drop(join_e);
    join_done(env::TaskId::new(e_tid));
    // `Join` 返回真 ⇒ 退出钩子（`gate::doom`）已跑完 ⇒ Q 当场就死了——不必重试。
    let q_dead = q.push(&msg).is_err();
    term.writeline(&format!("cascade: task-exit q.dead={q_dead}"));

    let ok = revoked && before && !after && b_dead && d_ok && a_ok && released && d_dead && q_dead;
    term.writeline(if ok { "cascade: ok" } else { "cascade: FAIL" });
}

/// 资源寿命自检（`reclaim` 命令）。
///
/// 三段判据：
///   1. 反复 unseal + release —— 泄漏则耗尽帧池（≈128 MB / 4 KB ≈ 3 万帧）；
///   2. 封印只归开辟者：他人 `Seal` 被拒；主人 `Seal` 后他人操作得 `Dead`；
///   3. 开辟者消亡 → 它开的资源随之回收（我手里的副本随 `doom` 失效）。
fn reclaim(term: &Terminal) {
    const ROUNDS: usize = 40_000;
    const WAIT: usize = 5_000;

    let rw = env::Permission::READ | env::Permission::WRITE;
    let msg = [0x5au8; 8];
    let mut buf = [0u8; 8];

    // ── 1：资源随最后一份能力回收 ──
    let mut failed_at = 0usize;
    for i in 1..=ROUNDS {
        match PolePie::unseal(4096) {
            Ok(p) => {
                let _ = p.release();
            }
            Err(_) => {
                failed_at = i;
                break;
            }
        }
    }
    term.writeline(&format!(
        "reclaim: unseal+release x{ROUNDS} failed_at={failed_at}"
    ));

    let me = match unit::self_id() {
        Ok(id) => id,
        Err(_) => {
            term.writeline("reclaim: no self id");
            return;
        }
    };

    // ── 2：封印只归开辟者 ──
    let a = match HolePie::unseal(HOLE_MTU_MAX) {
        Ok(p) => p,
        Err(_) => {
            term.writeline("reclaim: unseal a failed");
            return;
        }
    };
    let c_slot: &'static [AtomicUsize; 1] = Box::leak(Box::new([AtomicUsize::new(0)]));
    let c_ptr = c_slot.as_ptr() as usize;
    let k_ready = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let k_sealed = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let k_ack = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;

    let join = unit::closure(move || {
        let _ = room::wait(k_ready, WAIT);
        let c = HolePie::from_token(unsafe {
            (*(c_ptr as *const AtomicUsize)).load(Ordering::Relaxed)
        });
        let denied = c.seal().is_err(); // 非开辟者 → 应被拒
        let _ = room::wake(k_ack);
        let _ = room::wait(k_sealed, WAIT);
        let dead = c.push(&msg).is_err(); // 主人封印后 → 应失效
        (denied, dead)
    });
    let c = match a.accord(join.id(), rw) {
        Ok(t) => HolePie::from_token(t),
        Err(_) => {
            drop(join);
            term.writeline("reclaim: accord failed");
            return;
        }
    };
    c_slot[0].store(c.token(), Ordering::Relaxed);
    let _ = room::wake(k_ready);
    let _ = room::wait(k_ack, WAIT);
    let owner_sealed = a.seal().is_ok();
    let _ = room::wake(k_sealed);
    let (denied, dead) = join.join();
    term.writeline(&format!(
        "reclaim: other.seal_denied={denied} owner.seal={owner_sealed} other.after={dead}"
    ));

    // ── 3：开辟者消亡 → 资源随之回收 ──
    let q_slot: &'static [AtomicUsize; 1] = Box::leak(Box::new([AtomicUsize::new(0)]));
    let q_ptr = q_slot.as_ptr() as usize;
    let k_open = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let k_q = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;
    let k_done = Box::leak(Box::new([0u8; 8])).as_ptr() as usize;

    let join_e = unit::closure(move || {
        let _ = room::wait(k_open, WAIT);
        if let Ok(r) = HolePie::unseal(HOLE_MTU_MAX)
            && let Ok(q) = r.accord(me, rw)
        {
            unsafe { (*(q_ptr as *const AtomicUsize)).store(q, Ordering::Relaxed) };
        }
        let _ = room::wake(k_q);
        // 等本侧检查完「退出前可用」再返回——否则 `doom` 会在检查之前就把 q 收走。
        let _ = room::wait(k_done, WAIT);
    });
    let e_tid = join_e.id();
    let _ = room::wake(k_open);
    let _ = room::wait(k_q, WAIT);
    let q = HolePie::from_token(q_slot[0].load(Ordering::Relaxed));
    let q_ok = q.push(&msg).is_ok();
    let _ = q.pull(&mut buf);
    let _ = room::wake(k_done);
    drop(join_e);
    join_done(env::TaskId::new(e_tid));
    // `Join` 返回真 ⇒ 退出钩子已跑完 ⇒ q 当场失效（无需重试）。
    let q_dead = q.push(&msg).is_err();
    term.writeline(&format!(
        "reclaim: owner-exit q.before={q_ok} q.after={q_dead}"
    ));

    let ok = failed_at == 0 && denied && owner_sealed && dead && q_ok && q_dead;
    term.writeline(if ok { "reclaim: ok" } else { "reclaim: FAIL" });
}

/// 身份伪造自检（`spoof` 命令）。
///
/// 修复前：目录按请求体里的回信 token 求 `vestor` 认人——**猜中别人的 token 即可
/// 冒充**。修复后：身份 = **内核在 `Push` 时盖章的发送者**，报文里的字段只当回信
/// 地址用，且必须**确实是该发送者授给目录的那一枚**。
///
/// 四段判据：
///   1. 内核盖章：自己推的消息，`pull_from` 回来的发送者是自己；
///   2. 正向对照：用自己那枚回信 token 注册 / 解绑**预约给本域**的名字 → 两次 `Ok`；
///   3. 攻击：把 `1..=200` 逐个当作「猜中的回信 token」发 `Unregister("echo")`
///      ——目录按 sender 认人，全部失败，echo 的名字仍在；
///   4. 攻击者收不到任何 `Ok`（回信地址不属于发送者即被丢弃）。
fn spoof(term: &Terminal) {
    const WAIT: usize = 1_000;
    const GUESS_MAX: usize = 200;
    const REPLY_AT: usize = env::dispatch::REPLY_AT;
    let rw = env::Permission::READ | env::Permission::WRITE;

    let me = unit::self_id().unwrap_or(0);

    // ── 1：内核盖章 ──
    let self_stamp = (|| -> Option<bool> {
        let h = HolePie::unseal(64).ok()?;
        h.push(b"x").ok()?;
        let mut b = [0u8; 64];
        let (_, from) = h.pull_from(&mut b).ok()?;
        Some(from.get() == me)
    })()
    .unwrap_or(false);

    let entry = HolePie::from_token(DIR_ENTRY.load(Ordering::Relaxed));
    let dir_id = match mail::owned(entry.token()) {
        Ok((_, owner)) => owner.get(),
        Err(_) => {
            term.writeline("spoof: no dir id");
            return;
        }
    };
    // 本任务自己的回信孔（攻击者身份就用它）。
    let mine = match HolePie::unseal(HOLE_MTU_MAX) {
        Ok(p) => p,
        Err(_) => {
            term.writeline("spoof: unseal failed");
            return;
        }
    };
    let at_dir = match mine.accord(dir_id, rw) {
        Ok(t) => t,
        Err(_) => {
            term.writeline("spoof: accord failed");
            return;
        }
    };
    let mut buf = [0u8; MSG_LEN];
    // `ms` = 等回复的上界：正路径用 WAIT，猜 token 时用 0（只探测、顺便排空）。
    let mut call = |req: &Request, reply_tok: usize, ms: usize| -> Option<Reply> {
        let mut msg = req.encode();
        msg[REPLY_AT..REPLY_AT + 8].copy_from_slice(&reply_tok.to_le_bytes());
        entry.push(&msg).ok()?;
        mine.pull_timeout(&mut buf, ms).ok()?;
        Reply::decode(&buf).ok()
    };

    // 入口门闩必须与回信孔分开：注销会**释放**目录侧那枚入口副本，若回信地址正是
    // 它，回复就无处可推（`unpublish` 先释放、回复后推）。
    let entry_hole = match HolePie::unseal(HOLE_MTU_MAX) {
        Ok(p) => p,
        Err(_) => {
            term.writeline("spoof: unseal failed");
            return;
        }
    };
    let entry_at_dir = match entry_hole.accord(dir_id, rw) {
        Ok(t) => t,
        Err(_) => {
            term.writeline("spoof: accord failed");
            return;
        }
    };

    // 每次探测都用**新会话**：攻击循环可能把本任务自己的回信槽灌进一条陈旧回复
    //（攻击者只能污染自己的孔——`reachable` 检查挡住了替他人收信）。
    let before = dir_session()
        .and_then(|d| d.discover("echo"))
        .unwrap_or(false);

    // ── 2：正向对照（root 预约给本域的名字，自己的回信 token）──
    let own_ok = match Name::new("shell") {
        Ok(n) => {
            let reg = call(
                &Request::Register {
                    name: n,
                    entry: env::PieToken::new(entry_at_dir),
                },
                at_dir,
                WAIT,
            );
            let unreg = call(&Request::Unregister { name: n }, at_dir, WAIT);
            matches!(reg, Some(Reply::Ok)) && matches!(unreg, Some(Reply::Ok))
        }
        Err(_) => false,
    };

    // ── 3：攻击——逐个猜回信 token ──
    if let Ok(name) = Name::new("echo") {
        for tok in 1..=GUESS_MAX {
            // 回复只可能落到被猜中的那枚 token 的孔里（攻击者看不见），此处探测
            // 仅用于排空本任务自己的回信槽——判据是「echo 还在不在」。
            let _ = call(&Request::Unregister { name }, tok, 0);
        }
    }
    let after = dir_session()
        .and_then(|d| d.discover("echo"))
        .unwrap_or(false);

    term.writeline(&format!(
        "spoof: stamp={self_stamp} own={own_ok} echo.before={before} echo.after={after} (guess {GUESS_MAX})"
    ));
    let ok = self_stamp && own_ok && before && after;
    term.writeline(if ok { "spoof: ok" } else { "spoof: FAIL" });
}

/// 名字权限自检（`name` 命令）。
///
/// 目录的表只能由**父域（root）的预约**产生：注册只能**填**已预约的行。判据六段：
///   1. 预约者注册自己的名字 → `Ok`，且 `discover` 为真；
///   2. 注销只摘实例：`discover` 转假（名字仍归本域）；
///   3. 非预约者注册别人的名字 → `Denied`；
///   4. 未预约的名字 → `NotFound`；
///   5. 实例门闩消亡（释放源门闩 → 目录侧副本随 `sire` 级联摘掉）→ 名字自动回到
///      「无实例」；
///   6. 死实例不锁名字：重新注册成功。
fn name(term: &Terminal) {
    let dir = match dir_session() {
        Ok(d) => d,
        Err(_) => {
            term.writeline("name: no session");
            return;
        }
    };
    let code = |r: env::EnvResult<()>| r.err().map(|e| e.into_source().code());

    // ── 1+2：预约者注册 / 注销只摘实例 ──
    let publish = match HolePie::unseal(HOLE_MTU_MAX) {
        Ok(h) => dir.register("shell", &h).is_ok(),
        Err(_) => false,
    };
    let visible = dir.discover("shell").unwrap_or(false);
    let unpublish = dir.unregister("shell").is_ok();
    let gone = !dir.discover("shell").unwrap_or(true);

    // ── 3+4：非预约者 / 未预约的名字（各用一枚门闩；末尾释放以清掉目录侧副本）──
    let (foreign, ghost) = match HolePie::unseal(HOLE_MTU_MAX) {
        Ok(h) => {
            let f = code(dir.register("echo", &h)) == Some(E_DENIED);
            let g = code(dir.register("ghost", &h)) == Some(E_NOT_FOUND);
            let _ = h.release();
            (f, g)
        }
        Err(_) => (false, false),
    };

    // ── 5：实例门闩消亡 → 目录侧副本随 sire 级联摘掉 → 名字回到「无实例」──
    let stale = match HolePie::unseal(HOLE_MTU_MAX) {
        Ok(h) => {
            let reg = dir.register("shell", &h).is_ok();
            let _ = h.release();
            reg && !dir.discover("shell").unwrap_or(true)
        }
        Err(_) => false,
    };

    // ── 6：死实例不锁名字；末尾注销，恢复干净状态 ──
    let reuse = match HolePie::unseal(HOLE_MTU_MAX) {
        Ok(h) => dir.register("shell", &h).is_ok(),
        Err(_) => false,
    };
    let clean = dir.unregister("shell").is_ok();

    term.writeline(&format!(
        "name: publish={publish} visible={visible} unpublish={unpublish} gone={gone} foreign={foreign} ghost={ghost} stale={stale} reuse={reuse}"
    ));
    let ok = publish && visible && unpublish && gone && foreign && ghost && stale && reuse && clean;
    term.writeline(if ok { "name: ok" } else { "name: FAIL" });
}

/// 各系统能力命令。全部输出经 `term`（唯一 console 出口）。
/// 返回 false = 退出（exit 命令）。
fn exec(cmd: &str, args: &[String], term: &Terminal) -> bool {
    match cmd {
        "help" => {
            term.writeline(
                "help / clock / ticks / alloc / echo / sleep / spawn / heir / hole / req / dir / cascade / reclaim / spoof / name / badslot / exit",
            );
        }
        "clock" => {
            let (s, n) = clock().unwrap_or((0, 0));
            term.writeline(&format!("clock {s}.{:09} sec", n));
        }
        "ticks" => {
            let t = chrono::ticks().unwrap_or(0);
            term.writeline(&format!("ticks {t}"));
        }
        "alloc" => {
            let addr = task::env::memory::allocate(4096).unwrap_or(0);
            term.writeline(&format!("alloc -> {addr:#x}"));
        }
        "echo" => {
            term.writeline(&args.join(" "));
        }
        "sleep" => {
            let ms = args
                .first()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            term.writeline(&format!("sleep {ms}ms"));
            let _ = sleep(Duration::from_millis(ms));
            term.writeline("woke");
        }
        "spawn" => {
            // 闭包 join：算 0..N。
            let n = args
                .first()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(1000);
            let sum = unit::closure(move || {
                let mut acc: u64 = 0;
                for i in 0..n {
                    acc = acc.wrapping_add(i);
                }
                acc
            })
            .join();
            term.writeline(&format!("spawnjoin -> {sum}"));
        }
        "heir" => {
            // 血缘枚举：我生的子域（heir）——先 count 再逐个取 TeamId。
            let n = heir_count().unwrap_or(0);
            if n == 0 {
                term.writeline("heir: none");
            } else {
                term.writeline(&format!("heir: {n} children"));
                for i in 0..n {
                    let tid = heir_at(i).unwrap_or(env::TeamId::new(0));
                    term.writeline(&format!("  heir[{i}] = team {tid:?}"));
                }
            }
        }
        "hole" => {
            let msg = b"hi from shell";
            let pie = HolePie::unseal(HOLE_MTU_MAX).unwrap();
            let mut buf = [0u8; 64];
            let mut m = [0u8; 64];
            m[..msg.len()].copy_from_slice(msg);
            pie.push(&m).ok();
            pie.pull(&mut buf).ok();
            term.writeline(&format!(
                "hole got {:?}",
                core::str::from_utf8(&buf).unwrap_or("?")
            ));
            pie.seal().ok();
        }
        "cascade" => {
            cascade(term);
        }
        "reclaim" => {
            reclaim(term);
        }
        "spoof" => {
            spoof(term);
        }
        "name" => {
            name(term);
        }
        "badslot" => {
            // 非法 envcall 槽位（`a7` 由 U 态完全控制）：空号 class 1 idx 5 +
            // **未分配**的 class 8 + 越界。判据 = 内核把调用号当**参数**拒掉
            // （a0 回负码）并续跑本任务，而不是 panic 打死整机。此处不走
            // `EnvCall` 解码（那正是要绕过的正常路径），直入 ABI 的唯一汇编入口
            // `trap`。
            let mut neg = 0usize;
            for slot in [0x1_0000_0005usize, 0x8_0000_0000, usize::MAX] {
                // SAFETY: 照 ABI 摆 slot + 6 参数；本调用只探错误路径，不依赖返回语义。
                let (a0, _a1) = unsafe { env::ecall::trap(slot, [0; 6]) };
                if (a0 as isize) < 0 {
                    neg += 1;
                }
            }
            term.writeline(&format!("badslot: {neg}/3 rejected, kernel alive"));
        }
        "req" => {
            // 走目录协议：Directory::open 取会话 → Connect("echo") 拿服务入口门闩
            // → Service::call 一次往返（echo 对载荷字节 +1）。
            let dir = match dir_session() {
                Ok(d) => d,
                Err(e) => {
                    term.writeline(&format!("req dir err: {e:?}"));
                    return true;
                }
            };
            let svc = match dir.connect("echo") {
                Ok(s) => s,
                Err(e) => {
                    term.writeline(&format!("req connect err: {e:?}"));
                    return true;
                }
            };
            let txt = args.first().map(|s| s.as_str()).unwrap_or("hello-service");
            let mut payload = [0u8; PAYLOAD_LEN];
            let bytes = txt.as_bytes();
            let n = bytes.len().min(PAYLOAD_LEN - 1);
            payload[..n].copy_from_slice(&bytes[..n]);
            match svc.call(&payload) {
                Ok(got) => {
                    term.writeline(&format!(
                        "req echo -> {:?}",
                        core::str::from_utf8(&got).unwrap_or("?")
                    ));
                }
                Err(e) => {
                    term.writeline(&format!("req echo err: {e:?}"));
                }
            }
            let _ = svc.disconnect();
        }
        "dir" => {
            // 目录协议：Discover（纯探测）+ Enumerate（按名排序分页）。
            let dir = match dir_session() {
                Ok(d) => d,
                Err(e) => {
                    term.writeline(&format!("dir open err: {e:?}"));
                    return true;
                }
            };
            match dir.discover("echo") {
                Ok(true) => term.writeline("discover echo -> found"),
                Ok(false) => term.writeline("discover echo -> not found"),
                Err(e) => term.writeline(&format!("dir discover err: {e:?}")),
            }
            let mut after: Option<String> = None;
            loop {
                match dir.list(after.as_deref()) {
                    Ok(Some(name)) => {
                        term.writeline(&format!("  {}", name.as_str()));
                        after = Some(String::from(name.as_str()));
                    }
                    Ok(None) => break,
                    Err(e) => {
                        term.writeline(&format!("dir list err: {e:?}"));
                        break;
                    }
                }
            }
        }
        "exit" => {
            term.writeline("bye");
            return false;
        }
        _ => {
            term.writeline(&format!("unknown: {cmd} (try help)"));
        }
    }
    true
}

#[unsafe(no_mangle)]
extern "C" fn main() {
    // 启动期握手：拿到目录请求门闩的句柄（身份由 Owned 从门闩自身求得）。
    let entry_token = match shake() {
        Ok(token) => token,
        Err(_) => task::env::control::panic(1),
    };
    DIR_ENTRY.store(entry_token, Ordering::Relaxed);

    let term = Terminal::default();
    term.clear();
    term.fg(Color::Green);
    term.writeline("SQware shell");
    term.reset();
    term.writeline("type 'help' for commands.");

    loop {
        term.fg(Color::Cyan);
        // readline 收纳 prompt：Terminal 内部打 prompt + 行编辑 + 清行重绘含 prompt。
        let line = match term.readline("sq > ") {
            Readline::Line(s) => s,
            Readline::Eof | Readline::Interrupt => {
                term.reset();
                continue;
            }
        };
        term.reset();
        let args = split(&line);
        if args.is_empty() {
            continue;
        }
        let cmd = args[0].clone();
        let rest = &args[1..];
        if !exec(&cmd, rest, &term) {
            break;
        }
    }

    room::exit()
}
