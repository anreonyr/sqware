#![no_std]
#![no_main]
//! uart — 串口驱动域（U 态，**两个线程，各等一条孔**）。
//!
//! ```text
//! 写线程（boot 线程）  等写请求孔（无界）→ put（THRE 忙等）→ 回 Ok
//! 读线程               排空 → push 投递孔 → 等 PLIC 会话孔（**有界兜底**）→ 取走线号
//! ```
//!
//! # 为什么是两个线程
//!
//! 两个来源都是**交互式**的：写的及时性与输入的延迟都被人感知，而内核一次只能等一条孔
//! （没有多路等待原语）。于是"谁等哪条孔"决定孔的划分——**两个等待者不能等同一条孔**
//! （先醒的那个会把别人的报文取走）。这条判据与 PLIC 侧那条是同一个（§8.1.5：它的报文
//! 是引导期事件，故它用"非阻塞探测 + 有界等待"；这里两个来源都等不起，故两个等待者）。
//!
//! # 读线程为什么要**每轮排空**，那 20 ms 又是什么
//!
//! 会话孔上的令牌是**提前叫醒**，不是"数据搬运的许可证"：FIFO 使能之后，不足触发阈值的
//! 零散字节**可能根本不产生中断**，只等令牌就会把它们永远留在 FIFO 里（实测：一个命令敲
//! 到第三个字符就断，`spawn` 永远等不到回车）。故次序是 `排空 → 投递 → 开门 → 有界等`，
//! 那一等在有中断时由内核 `try_push` 当场唤醒，没中断时只是兜底——与 `prog-plic` 同值
//! （20 ms）同理由。
//!
//! # 唤醒链（两跳，每跳一次显式唤醒）
//!
//! PLIC 按**线号**取持有者（本域）→ 把线号投进本域登记的那条会话孔 ⇒ 读线程**当场被唤醒**；
//! 读线程排空设备 → 把字节投进**投递孔** ⇒ console 的输入线程被唤醒。
//! 投递孔单槽 + `push` 阻塞 ⇒ **背压**：下游不收，本域就停在那一句上，不再排空。
//!
//! # 设备与名字都从 root 来
//!
//! 四件配给（次序即下方 `Pier::pull` 的次序）：目录请求门闩 → **设备门闩** → **设备名**
//! → **投递孔的 WRITE 副本**。设备名不是本域硬编码的（名字的账在 boot 的配对块里，
//! root 是它的读者）；本域也不认识线号——`register` 只报名字，线号由 PLIC 从设备树解出。
//!
//! # 参数表
//!
//! 启动失败一律 `exit_with(n)`：它是**配置错误**（配给次序对不上 / 设备不在），
//! 装作起来了只会让现象变成"输入静默不响"——那是最难查的一类。

extern crate alloc;
// 本包 lib 提供 `_start` + panic_handler；必须真的链接它，`use` 只带符号不算。
extern crate programs;

use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::format;

use env::{HoleDir, TeamId};
use protocol::dispatch::client::Directory;
use protocol::irq;
use protocol::uart::{self, Action, DELIVER_MTU, MSG_LEN, SERVICE, Status};
use runtime::core::handshake::{self, Pier, Quay};
use runtime::core::lock::Lock;
use runtime::core::port::{Access, Policy, ship};
use runtime::env::mail::{self, HolePie, PolePie};
use runtime::env::task as utask;

use programs::uart::Uart;

/// 等中断的上界（毫秒）。**不是轮询周期**：有中断时这一等由内核的 `try_push` 当场唤醒
/// （立即返回），无中断时它只是兜底——FIFO 使能后不足阈值的零散字节可能不产生中断，
/// 只有这一等能把它们捞上来。与 `prog-plic` 同值同理由。
const IRQ_WAIT_MS: usize = 20;

/// 等设备名字的上界（毫秒）：root 按同一条下行通道递过来，故这一等只是一次握手。
const NAME_WAIT_MS: usize = 1000;

/// 设备视图：`Uart` 是 `Copy`，故两个线程各取一份（同一张页表，**不需要锁**）。
static UART: Lock<Option<Uart>> = Lock::new(None);

/// 交给读线程的两枚门闩（主线程写、读线程读；`Hatch` 是同步点）。
///
/// 门闩是 **per-task** 的：主线程 `Accord` 出去拿到的是**对方表里**的号，只能经共享内存
/// 交接（`Spawn` 恒产 `Held` ⇒ 「先 `Accord`、再写静态、最后 `Hatch`」的次序天然成立）。
static SESSION: AtomicUsize = AtomicUsize::new(0);
static DELIVER: AtomicUsize = AtomicUsize::new(0);

/// 从 root 配给的名字孔里取回设备名。
fn take_name(hole: HolePie) -> Option<env::Name> {
    let mut buf = [0u8; env::NAME_LEN];
    let n = hole.pull_timeout(&mut buf, NAME_WAIT_MS).ok()?;
    let name = env::Name::new(core::str::from_utf8(&buf[..n]).ok()?).ok()?;
    // 名字只有一个读者，读完就把这一份放下（`root` 手里那份是它自己的账）。
    let _ = mail::release(hole.token());
    Some(name)
}

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 靠泊 + 自建控制孔（只给父域；与请求孔分离——父域拿不到请求队列）。
    let up = match handshake::moor() {
        Ok(u) => u,
        Err(_) => runtime::env::room::exit_with(1),
    };
    let down = match HolePie::unseal() {
        Ok(h) => h,
        Err(_) => runtime::env::room::exit_with(2),
    };
    let sire = match utask::sire() {
        Ok(t) => t,
        Err(_) => runtime::env::room::exit_with(3),
    };
    let at_parent = match ship(&down, sire, Access::READ | Access::WRITE, Policy::NONE) {
        Ok(to) => to.token(),
        Err(_) => runtime::env::room::exit_with(4),
    };
    if Quay::new(at_parent).push(&up).is_err() {
        runtime::env::room::exit_with(5);
    }

    // 2. 收目录请求门闩 → 注册服务名。**请求孔由本线程开**：客户端认服务靠
    //    `Reserve(entry).owner`（开辟者），而回信孔要落进开辟者那张表里。
    let pier = match Pier::pull(&down) {
        Ok(p) => p,
        Err(_) => runtime::env::room::exit_with(6),
    };
    let dir = match Directory::open(HolePie::from_token(pier.token())) {
        Ok(d) => d,
        Err(_) => runtime::env::room::exit_with(7),
    };
    let entry = match HolePie::unseal() {
        Ok(h) => h,
        Err(_) => runtime::env::room::exit_with(8),
    };
    if dir.register(SERVICE, &entry).is_err() {
        runtime::env::room::exit_with(9);
    }

    // 3. 收设备：`Open` 之后那段物理区就在本域页表里，此后读写寄存器就是 load/store。
    let dev = match Pier::pull(&down) {
        Ok(p) => p,
        Err(_) => runtime::env::room::exit_with(10),
    };
    let uart = match Uart::open(PolePie::from_token(dev.token())) {
        Ok(u) => u,
        Err(_) => runtime::env::room::exit_with(11),
    };

    // 4. 收设备名：**root 先写属主、再交名字**（客户端一拿到名字就会去登记，而登记读的
    //    正是刚写的那一行；同一条 FIFO 队列 ⇒ 先推的先被处理）。
    let name_pier = match Pier::pull(&down) {
        Ok(p) => p,
        Err(_) => runtime::env::room::exit_with(12),
    };
    let Some(name) = take_name(HolePie::from_token(name_pier.token())) else {
        runtime::env::room::exit_with(13);
    };

    // 5. 收投递孔（root 开辟并留源副本；本域拿 WRITE 那一份）。
    let deliver_pier = match Pier::pull(&down) {
        Ok(p) => p,
        Err(_) => runtime::env::room::exit_with(14),
    };
    let deliver = HolePie::from_token(deliver_pier.token());

    // 6. 登记那条线：会话门闩由本线程造，READ 副本给读线程（"客户端递出门闩"）。
    let session = match HolePie::unseal() {
        Ok(h) => h,
        Err(_) => runtime::env::room::exit_with(15),
    };
    let line = match irq::Line::connect(&dir) {
        Ok(l) => l,
        Err(_) => runtime::env::room::exit_with(16),
    };
    if !matches!(line.register(&name, &session), Ok(irq::Ack::Ok)) {
        runtime::env::room::exit_with(17);
    }

    // 7. 读线程：`Spawn`（Held，不跑）→ 交两枚门闩与设备视图 → `Hatch`。
    let entry_va = read_loop as extern "C" fn() -> ! as usize;
    let read_task = match utask::spawn(TeamId(0), entry_va, &[], 0) {
        Ok(t) => t,
        Err(_) => runtime::env::room::exit_with(18),
    };
    let at_read = match ship(&session, read_task, Access::READ, Policy::NONE) {
        Ok(to) => to.token(),
        Err(_) => runtime::env::room::exit_with(19),
    };
    let deliver_at_read = match ship(&deliver, read_task, Access::WRITE, Policy::NONE) {
        Ok(to) => to.token(),
        Err(_) => runtime::env::room::exit_with(20),
    };
    UART.with(|u| *u = Some(uart));
    SESSION.store(at_read.get(), Ordering::Release);
    DELIVER.store(deliver_at_read.get(), Ordering::Release);
    if utask::hatch(read_task).is_err() {
        runtime::env::room::exit_with(21);
    }

    // 8. 上线。此刻 console 还没起，输出通路只有**本域自己那台设备**（与 root 上线前
    //    用门闩打印同一款）——故只在这一处说一句：线登记成了。
    announced(&uart, &name);

    // 9. boot 线程此后就是**写线程**。
    write_loop(entry, uart)
}

/// 登记成了一句：**走本域自己的设备**（此刻控制台还没起来，也只能这么打）。
///
/// 它是"这个实例拿到了 `Ok`"的唯一可见证据——`Taken` / `NotYours` / `Unclaimed` 都不会
/// 走到这里（那些情形本域当场收场）。长度控制在一条报文以内，日志里是原子的一行。
fn announced(uart: &Uart, name: &env::Name) {
    uart.put(format!("uart: line {} ok\n", name.as_str()).as_bytes());
}

/// 写线程：**等写请求孔（无界）→ 写设备 → 回执**。
///
/// 无界是硬的：请求是事件不是节拍，本线程没有别的活。门没了（副本被摘 / 被封印）即收场
/// ——服务的门没了，活也就没了（与 `doom` 服务线程同款）。
fn write_loop(entry: HolePie, uart: Uart) -> ! {
    let mut msg = [0u8; MSG_LEN];
    loop {
        let Ok(n) = entry.pull(&mut msg) else {
            runtime::env::room::exit_with(0);
        };
        match uart::serve(&msg[..n]) {
            // 回执在**写完之后**才推：`Ok` 的含义是"字节已进设备"，不是"收到了"。
            Action::Write { reply, bytes } => {
                uart.put(bytes);
                let _ = HolePie::from_token(reply.get()).push(&[Status::Ok.byte()]);
            }
            Action::Reply { reply, status } => {
                let _ = HolePie::from_token(reply.get()).push(&[status.byte()]);
            }
            Action::Ignore => {}
        }
    }
}

/// 读线程：**排空设备 → 投给下游 → 等 PLIC 会话孔 → 取走线号**（每轮都排空）。
///
/// 投不出去就是**下游没了**：丢这批字节，**线不动**——线是本域的（设备在本域手里），
/// 不随 console 的死而收；下游重发回来接上投递孔即可继续（`docs/driver.md` §3.2.9 的
/// "一次失败即收线"在这里换了主语：那是 PLIC 对**持有者**的判据）。
extern "C" fn read_loop() -> ! {
    let Some(uart) = UART.with(|u| *u) else {
        runtime::env::room::exit_with(22);
    };
    let session = HolePie::from_token(SESSION.load(Ordering::Acquire));
    let deliver = HolePie::from_token(DELIVER.load(Ordering::Acquire));
    let mut tok = [0u8; irq::LINE_LEN];
    let mut chunk = [0u8; DELIVER_MTU];
    // 一次性读数：**"投递把本域叫醒"这件事的唯一可见证据**（判据见 `scripts/examine.nu`
    // 的 `IRQ_MARKER` 段）。为什么需要它：本线程那一等有 20 ms 兜底，故"输入能用"本身
    // 证明不了中断链在场——兜底轮询同样能让输入工作。
    let mut seen_irq = false;
    // 起手先关门：循环的第一步就是排空（门关着）。
    uart.mask_rx();
    loop {
        // ① 排空设备（门关着）：中断只说"有"，不说"有多少"——FIFO 里可能攒了好几个字节。
        //    **每轮都排**，而不是"等到了令牌才排"：令牌只是**提前叫醒**，它不是数据搬运的
        //    许可证。FIFO 使能后不足阈值的零散字节可能不产生中断，只有兜底那一等能把它们
        //    捞上来（旧 console 的 `drain → unmask → Wait → Pull → mask` 就是这个道理）。
        let mut n = 0;
        while n < chunk.len() {
            match uart.try_get() {
                Some(b) => {
                    chunk[n] = b;
                    n += 1;
                }
                None => break,
            }
        }
        // ② 投给下游：可能阻塞（背压），但会话槽此刻是空的、门也关着 ⇒ 堵在这里不会把
        //    中断链锁死（PLIC 的投递是阻塞 `push`，堵住它就会连带关掉内核那道闸门）。
        if n > 0 {
            let _ = deliver.push(&chunk[..n]);
        }
        // ③ 开门（**重新武装**：此刻若 FIFO 仍有数据，线上跳 ⇒ PLIC 重新置 pending）→
        //    等下一次投递。**有界**：有中断时由内核的 `try_push` 当场唤醒（立即返回），
        //    没中断时这一等只是兜底——不能是无穷，否则零散字节没人捞。
        uart.unmask_rx();
        let _ = session.wait(HoleDir::Pull, IRQ_WAIT_MS);
        // ④ 一被叫醒先关门，再把会话槽取空：PLIC 的投递是**单槽**，槽满它就一直卡在
        //    那一句上（内核往 irq 孔推令牌随即 `Busy` ⇒ 关闸门 ⇒ 三方互等）。
        uart.mask_rx();
        while session.pull_timeout(&mut tok, 0).is_ok() {
            if !seen_irq {
                seen_irq = true;
                // 只打一次：每键一行会把屏幕刷满（与 `prog-plic` 的那条同款）。
                uart.put(b"uart: irq ok\n");
            }
        }
    }
}
