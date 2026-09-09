#![no_std]
#![no_main]

//! shell — 命令解释器，经 Terminal（term 模块）显示、读命令并分发给系统能力。
//!
//! 分层：本 bin 是 Shell；`task::term::{Terminal, Readline}` 是渲壳 + 行编辑宿主。
//! **Terminal 是唯一 console 出口**——Shell 的一切输出经 `Terminal::put`、一切
//! 输入经 `Terminal::readline`，不再直接 `io::put`。
//!
//! 命令（系统能力巡演）：
//!   help  — 列命令
//!   clock — 读全局时钟（ChronoCall::Clock）
//!   ticks — 读 timebase 刻度
//!   alloc — 堆分配一页（MemoryCall::Allocate）
//!   echo  — 回显参数
//!   sleep — 阻塞 N 毫秒（RoomCall::Park）
//!   spawn — 派一个算 0..N 的闭包子任务并 join（TaskCall::Spawn）
//!   hole  — Hole 通道自测（unseal/push/pull/seal）
//!   cascade — 派生级联自检（三跳撤销 / 无关分支 / release 级联 / 任务消亡级联）
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

use task::core::handshake::{self, Pier, Quay};
use task::core::service::{Directory, PAYLOAD_LEN};
use task::core::unit;
use task::env::{
    chrono::{self, clock},
    mail::{HOLE_MTU_MAX, HolePie},
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

/// 派生级联自检（`cascade` 命令）。
///
/// 四段判据：
///   1. 三跳 A→B→C：撤销中间那跳 B，末端 C 必须失效；
///   2. 无关分支 D 不受波及，资源本体 A 仍可用；
///   3. `release` 同样级联：放下 A → D 随之下线；
///   4. 任务消亡级联：closure 授出的 Q 随它回收而失效（退出钩子 `gate::doom`）。
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
    // 等该任务回收。注意 `join` 在目标已 Reaped 时**当场返回**，而退出钩子
    // （`gate::doom`）可能在返回之后才跑完——故此处有界等待（最多 500 ms）。
    let _ = task_join(env::TaskId::new(e_tid), WAIT);
    let mut q_dead = false;
    for _ in 0..50 {
        if q.push(&msg).is_err() {
            q_dead = true;
            break;
        }
        let _ = q.pull(&mut buf);
        let _ = sleep(Duration::from_millis(10));
    }
    term.writeline(&format!("cascade: task-exit q.dead={q_dead}"));

    let ok = revoked && before && !after && b_dead && d_ok && a_ok && released && d_dead && q_dead;
    term.writeline(if ok { "cascade: ok" } else { "cascade: FAIL" });
}

/// 各系统能力命令。全部输出经 `term`（唯一 console 出口）。
/// 返回 false = 退出（exit 命令）。
fn exec(cmd: &str, args: &[String], term: &Terminal) -> bool {
    match cmd {
        "help" => {
            term.writeline(
                "help / clock / ticks / alloc / echo / sleep / spawn / heir / hole / req / dir / exit",
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
