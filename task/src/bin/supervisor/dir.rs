#![no_std]
#![no_main]
//! dir — 服务目录（S 态 supervisor 域，**两个线程**）。
//!
//! ```text
//! 主线程      H 服务循环（pull 请求 → 认人 → 处理 → push 回复）
//! 控制线程    pull(C) → 预约（若报文带名字）→ H.accord(who, R|W) → push(上行孔, Referred)
//! ```
//!
//! **为什么两个线程**：目录要同时听两条输入通道（客户端的请求孔 `H`、父域的引入
//! 孔 `C`），而 `Wait` 一次只能等一条孔——单线程 park 在 `H` 上就接不到引入请求。
//! 控制面与数据面分开，主线程的循环一字未改。
//!
//! **两张表合一**：注册表即预约表（见 `task::core::directory`）。控制线程要往里
//! 写预约、主线程要读写，故用 `Lock<Directory>` 串起来；控制线程的处理次序是
//! **先预约、再开门**——客户端拿到门闩时预约必已就位。
//!
//! **自开门闩是硬规则**：客户端用 `MailCall::Owned` 从门闩副本的 `owner` 求目录
//! task id；若由他人代开，客户端会把回信 hole 授给代开者（见 `docs/dispatch.md`）。
//!
//! **认人**：`Pull` 一并交回**内核盖章的发送者**（`Push` 时写入，报文伪造不了）；
//! 请求 `[49..57]` 只是回信地址，且必须**确实是该发送者授给本域的那一枚**。
//!
//! 注册表逻辑在 `task::core::directory`；协议规范见 `docs/dispatch.md`。

extern crate alloc;

use core::sync::atomic::{AtomicUsize, Ordering};

use env::dispatch::{MSG_LEN, REPLY_AT};
use env::{Permission, TeamId};
use task::core::directory::{Directory, release_pie, vestor_of};
use task::core::handshake::{self, Quay, Refer, Referred};
use task::core::lock::Lock;
use task::env::mail::HolePie;
use task::env::task as utask;

/// 控制线程的三枚门闩（主线程写、控制线程读；`Hatch` 是同步点）。
///
/// 同域两线程共享地址空间，但**门闩是 per-task 的**：主线程 `Accord` 出去拿到的是
/// 对方表里的 token，只能经共享内存交接。`Spawn` 恒产 `Held`，故「先 `Accord`、再写
/// 静态、最后 `Hatch`」的次序天然成立——控制线程读到的必然是写好的值。
static CTRL: [AtomicUsize; 3] = [const { AtomicUsize::new(0) }; 3];

/// 注册表：控制线程写预约、主线程读写，故必须跨线程互斥。
static DIR: Lock<Directory> = Lock::new(Directory::new(vestor_of, release_pie));

/// 控制线程：接引入请求 → **预约** → 亲授目录请求门闩 → 回报。
#[unsafe(no_mangle)]
extern "C" fn control_main() -> ! {
    let entry = HolePie::from_token(CTRL[0].load(Ordering::Relaxed));
    let control = HolePie::from_token(CTRL[1].load(Ordering::Relaxed));
    let up = HolePie::from_token(CTRL[2].load(Ordering::Relaxed));
    loop {
        let refer = match Refer::pull(&control) {
            Ok(r) => r,
            // 控制孔死亡（root 已走）：无处可听，硬失败。
            Err(_) => task::env::control::panic(20),
        };
        // 先预约、再开门：否则客户端可能在预约落表前就注册。
        if let Some(name) = refer.name() {
            DIR.with(|d| d.reserve(name, refer.who().get()));
        }
        let token = entry
            .accord(refer.who(), Permission::READ | Permission::WRITE)
            .unwrap_or(0);
        if Referred::new(token).push(&up).is_err() {
            task::env::control::panic(21);
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 靠泊：认父域开的上行孔。
    let up = match handshake::moor() {
        Ok(u) => u,
        Err(_) => task::env::control::panic(1),
    };
    // 2. 自建请求门闩——服务自己开自己的门。
    let entry = match HolePie::unseal(MSG_LEN) {
        Ok(h) => h,
        Err(_) => task::env::control::panic(2),
    };
    // 3. 自建控制孔（只给父域），与请求门闩分离——父域拿不到请求队列。
    let control = match HolePie::unseal(handshake::REFER_MTU) {
        Ok(h) => h,
        Err(_) => task::env::control::panic(3),
    };
    let sire = match utask::sire() {
        Ok(t) => t,
        Err(_) => task::env::control::panic(4),
    };
    let at_parent = match control.accord(sire, Permission::READ | Permission::WRITE) {
        Ok(t) => t,
        Err(_) => task::env::control::panic(5),
    };

    // 4. 控制线程：先产（Held，不跑）→ 交接三枚门闩 → 放行。
    let entry_va = control_main as extern "C" fn() -> ! as usize;
    let ctrl = match utask::spawn(TeamId(0), entry_va, &[], 0) {
        Ok(t) => t,
        Err(_) => task::env::control::panic(6),
    };
    let h2 = match entry.accord(
        ctrl,
        Permission::READ | Permission::WRITE | Permission::VEST,
    ) {
        Ok(t) => t,
        Err(_) => task::env::control::panic(7),
    };
    let c2 = match control.accord(ctrl, Permission::READ | Permission::WRITE) {
        Ok(t) => t,
        Err(_) => task::env::control::panic(8),
    };
    let u2 = match up.accord(ctrl, Permission::READ | Permission::WRITE) {
        Ok(t) => t,
        Err(_) => task::env::control::panic(9),
    };
    CTRL[0].store(h2, Ordering::Relaxed);
    CTRL[1].store(c2, Ordering::Relaxed);
    CTRL[2].store(u2, Ordering::Relaxed);
    if utask::hatch(ctrl).is_err() {
        task::env::control::panic(10);
    }

    // 5. 报到：把控制孔在父侧的句柄交给 root（root 据此转达引入请求）。
    if Quay::new(at_parent).push(&up).is_err() {
        task::env::control::panic(11);
    }

    // 6. 服务循环。
    let mut msg = [0u8; MSG_LEN];
    loop {
        // 身份 = **内核盖章的发送者**（`Pull` 一并交回），不信任报文里的任何字段。
        let Ok((_, from)) = entry.pull_from(&mut msg) else {
            continue;
        };
        let caller = from.get();
        let reply_token =
            usize::from_le_bytes(msg[REPLY_AT..REPLY_AT + 8].try_into().unwrap_or([0u8; 8]));
        // 回信地址必须**确实是 caller 授给本域的那一枚**——否则丢弃回复
        //（防「替他人收信」：把别人的回信 token 塞进自己的请求）。
        let reachable = vestor_of(reply_token) == Some(caller);
        let out = DIR.with(|dir| dir.serve(caller, &msg)).encode();
        if !reachable {
            continue;
        }
        // 推回调用方自带的回信 hole；槽满则挂起等对侧取走，Dead 即丢。
        let _ = HolePie::from_token(reply_token).push(&out);
    }
}
