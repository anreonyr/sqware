#![no_std]
#![no_main]
//! plic — 中断线驱动（S 态 supervisor 域，**一个线程**）。
//!
//! ```text
//! 循环     报文（探测：登记 / 写属主）→ irq 门闩（有界等待）→ claim → 投线号 → complete
//! ```
//!
//! # 它认识什么、不认识什么
//!
//! 它认识 PLIC 的寄存器布局与设备树的绑定（**名字 → 线号**）；它**不认识任何设备**
//! ——不知道 `serial@10000000` 后面是串口还是网卡，只把"哪条线响了"投给持有那条线的
//! 客户端（`docs/driver.md` §3.2.6：名字 → (线号, 属主, 门闩) 的表活在**本域的内存**
//! 里，内核不参与）。
//!
//! # 内核在这一整条路上出现两次，且都不认识设备
//!
//! `外部中断 → trap_handler 推空令牌进 irq 门闩`（内核只知道"有外部中断"），
//! 剩下全在这里：`claim`（从 PLIC 领线号）→ 投递 → `complete`。整链两跳
//! （§3.2.4）。**没有 ack**：客户端下一次进门时 mask 自己的 `IER`、出门再开门
//! （§3.2.5），静音不需要 PLIC 参与。
//!
//! # 为什么只有一个线程
//!
//! 投递必须由**持有客户端门闩的那个 task** 做（门闩是 per-task 的——console 服务
//! 为这件事踩过坑并写进了它的模块头）。报文带来的门闩落在收报文的任务表里，
//! 故收报文与投递必须同任务。于是循环里两件事共处：报文用**非阻塞探测**（它们是
//! 引导期事件，晚 20 ms 无所谓），中断用**有界等待**（无界会把报文饿死）。
//! 中断路径本身不轮询：内核的 `try_push` 会唤醒站点。
//!
//! # 线号从哪来
//!
//! **名字的函数**（`docs/driver.md` §12 甲）：设备树的 `interrupts` ×
//! `interrupt-parent`（指向本控制器的 phandle）⇒ 线号。客户端**不报线号**——报文里
//! 也没有这个字段，它只报名字（`protocol::irq`）。于是"这条线是不是你的"这个问题在
//! 本域只有一条判据：名字的属主是不是它（属主**只能由 root 写**）。
//!
//! context 序号则来自 `interrupts-extended` 的**项序**（RISC-V 的中断控制器绑定），
//! 而 `cell == 9` 是 S 模式外部中断、`11` 是 M 模式——**认 9 不认 11**（认错就是把线
//! 交给固件）。**不硬算 `2h+1`**（§7.2）。

extern crate alloc;
// 本包 lib 提供 `_start` + panic_handler；必须真的链接它，`use` 只带符号不算。
extern crate programs;

use alloc::vec::Vec;

use env::HoleDir;
use env::Permission;
use programs::lines::Lines;
use protocol::console::Console;
use protocol::dispatch::client::Directory;
use protocol::irq;
use runtime::core::handshake::{self, Pier, Quay};
use runtime::core::lock::Lock;
use runtime::env::mail::{self, AnyPie as _, HolePie, PolePie};

/// `irq` 门闩的等待上界（毫秒）——**不能是无穷**：注册报文要有人听（见模块头）。
/// 中断路径不受它影响（槽满/有信都立即唤醒）。
const IRQ_WAIT_MS: usize = 20;

/// S 模式外部中断的中断号（`interrupts-extended` 里的 cell 值）。
const EXT_S: u32 = 9;

/// 一条线的优先级：恒 1（0 = 静音，见 [`Plic::disable`]）。它不再是客户端的字段——
/// 这个量没有第二个取值，问客户端等于让它替本域做设备侧的决定。
const LINE_PRIORITY: u32 = 1;

/// PLIC 的寄存器视图（一段有主的、可映射的内存——`docs/driver.md` §1）。
struct Plic {
    base: usize,
    /// 本控制器有多少条线（设备树 `riscv,ndev`）——按它拒绝越界的注册。
    ndev: u32,
    /// 所有 **S 外部** context 序号（每颗 hart 一个）。
    contexts: Vec<u32>,
}

/// PLIC 寄存器偏移（SiFive PLIC 布局；`reg` 给的是整块 0x600000）。
const PRIORITY: usize = 0x0000_0000;
const ENABLE: usize = 0x0000_2000;
const ENABLE_STRIDE: usize = 0x80;
const CONTEXT: usize = 0x0020_0000;
const CONTEXT_STRIDE: usize = 0x1000;
const THRESHOLD: usize = 0x00;
const CLAIM: usize = 0x04;

impl Plic {
    /// 开闩 + 读设备树：把"我是谁、我有哪些 context、这台机器上有哪些中断源"问清楚。
    ///
    /// 判据 = `interrupt-controller` 且 `compatible` 里含 `plic` 的节点。**解释设备
    /// 树是驱动的事**（内核只原样搬运自描述，§3.1.3），故这里可以按 compatible 认。
    ///
    /// 返的第二件是**线表**：它由同一棵树建出来（名字 → 线号），本域此后只认名字。
    fn open(pole: PolePie, dtb: PolePie) -> Option<(Self, Lines)> {
        let base = pole.open().ok()?;
        let dtb_va = dtb.open().ok()?;
        // SAFETY: DTB 门闩把设备树本体只读借映进了本域；`fdt` 只读它。
        let fdt = unsafe { fdt::Fdt::from_ptr(dtb_va as *const u8) }.ok()?;
        let node = fdt.all_nodes().find(|n| {
            n.property("interrupt-controller").is_some()
                && n.compatible()
                    .is_some_and(|c| c.all().any(|s| s.contains("plic")))
        })?;
        let ndev = node
            .property("riscv,ndev")
            .and_then(|p| p.as_usize())
            .unwrap_or(0) as u32;
        // 每项 = <目标 phandle, 中断号…>；中断号的字节数由 `#interrupt-cells` 定。
        let cells = node
            .property("#interrupt-cells")
            .and_then(|p| p.as_usize())
            .unwrap_or(1);
        let mut contexts = Vec::new();
        let prop = node.property("interrupts-extended")?;
        let stride = 4 + 4 * cells;
        for (i, entry) in prop.value.chunks_exact(stride).enumerate() {
            let cell = u32::from_be_bytes(entry[4..8].try_into().ok()?);
            if cell == EXT_S {
                contexts.push(i as u32);
            }
        }
        // 线表与控制器同源：同一个 `ndev` 只读一次（它就是"这台控制器有几条线"）。
        let lines = Lines::from_tree(&fdt, &node, ndev);
        Some((
            Self {
                base,
                ndev,
                contexts,
            },
            lines,
        ))
    }

    /// 使能一条线：`priority` 一条、**每个 S context** 各一份 enable。
    ///
    /// 为什么每个 context：中断要能在**任意一颗 hart** 上被取到（不引入任务亲和性，
    /// §3.2.7）。阈值恒 0（不卡仲裁）。
    fn enable(&self, line: u32, priority: u32) {
        self.write(PRIORITY + 4 * line as usize, priority);
        for &ctx in &self.contexts {
            let at = CONTEXT + CONTEXT_STRIDE * ctx as usize;
            self.write(at + THRESHOLD, 0);
            let e = ENABLE + ENABLE_STRIDE * ctx as usize + 4 * (line / 32) as usize;
            let bits = self.read(e) | 1 << (line % 32);
            self.write(e, bits);
        }
    }

    /// 关一条线（**收线**用，§12 ②）：`priority = 0`。
    ///
    /// 这是**可逆静音**，不是把线拆掉（§7.2 量到的语义）：`pending` 照旧置位，但
    /// `claim` 恒 0 ⇒ 本域不再投递它；重新登记时 [`Plic::enable`] 把优先级写回
    /// [`LINE_PRIORITY`]。enable 位留着——线号与设备的绑定没变，变的只是"现在有没有
    /// 人接"。
    fn disable(&self, line: u32) {
        self.write(PRIORITY + 4 * line as usize, 0);
    }

    /// 领一条线号；0 = 没有可领的（**不是错误**：另一颗 hart 的 context 可能已经
    /// 把它领走了——`claim` 原子清挂起，故第二个 `claim` 拿到 0）。
    fn claim(&self) -> u32 {
        let ctx = match self.contexts.first() {
            Some(&c) => c as usize,
            None => return 0,
        };
        self.read(CONTEXT + CONTEXT_STRIDE * ctx + CLAIM)
    }

    /// 结一条线（**不是重武装点**：`complete` 不会把中断带回来，§7.2）。
    fn complete(&self, line: u32) {
        let ctx = match self.contexts.first() {
            Some(&c) => c as usize,
            None => return,
        };
        self.write(CONTEXT + CONTEXT_STRIDE * ctx + CLAIM, line);
    }

    fn read(&self, off: usize) -> u32 {
        // SAFETY: `base` 是本域已映射的 PLIC 页；偏移落在 `reg` 声明的区间内。
        unsafe { core::ptr::read_volatile((self.base + off) as *const u32) }
    }

    fn write(&self, off: usize, v: u32) {
        // SAFETY: 同上，只写控制器寄存器。
        unsafe { core::ptr::write_volatile((self.base + off) as *mut u32, v) }
    }
}

/// 线表：**名字 → 线号 + 属主 + 活实例**。只在本域的内存里（§3.2.6）。
///
/// 它替掉了此前那张"线号 → 会话门闩"的表：那张表把权威放在了客户端自报的线号上
/// （任何域报一条 `line ≤ ndev` 就能把别人的线抢走，§12 的读数）。现在线号由本域从
/// 设备树解出来，客户端只能证明"名字是我的"。
static LINES: Lock<Option<Lines>> = Lock::new(None);

/// 设备与门闩：开一次、之后只读（`Lock` 只是为了让静态可写一次）。
static PLIC: Lock<Option<Plic>> = Lock::new(None);

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 靠泊 + 自建控制孔（只给父域）。
    let up = match handshake::moor() {
        Ok(u) => u,
        Err(_) => runtime::env::room::exit_with(1),
    };
    let down = match HolePie::unseal(handshake::MTU) {
        Ok(h) => h,
        Err(_) => runtime::env::room::exit_with(2),
    };
    let sire = match runtime::env::task::sire() {
        Ok(t) => t,
        Err(_) => runtime::env::room::exit_with(3),
    };
    let at_parent = match down.accord(sire, Permission::READ | Permission::WRITE) {
        Ok(t) => t,
        Err(_) => runtime::env::room::exit_with(4),
    };
    if Quay::new(at_parent).push(&up).is_err() {
        runtime::env::room::exit_with(5);
    }

    // 2. 收配给：目录请求门闩 → 注册本服务的名字。
    let pier = match Pier::pull(&down) {
        Ok(p) => p,
        Err(_) => runtime::env::room::exit_with(6),
    };
    let dir = match Directory::open(HolePie::from_token(pier.token())) {
        Ok(d) => d,
        Err(_) => runtime::env::room::exit_with(7),
    };
    ENTRY.with(|e| *e = Some(pier.token().get()));
    // 自建请求孔：客户端往它推动词（`protocol::irq` 的定长报文，故 `mtu` 就是它）。
    let entry = match HolePie::unseal(irq::LEN) {
        Ok(h) => h,
        Err(_) => runtime::env::room::exit_with(8),
    };
    if dir.register(irq::SERVICE, &entry).is_err() {
        runtime::env::room::exit_with(9);
    }

    // 3. 收三枚门闩（root 按序配给）：PLIC、设备树、`irq`。
    let plic_pier = match Pier::pull(&down) {
        Ok(p) => p,
        Err(_) => runtime::env::room::exit_with(10),
    };
    let dtb_pier = match Pier::pull(&down) {
        Ok(p) => p,
        Err(_) => runtime::env::room::exit_with(11),
    };
    let irq_pier = match Pier::pull(&down) {
        Ok(p) => p,
        Err(_) => runtime::env::room::exit_with(12),
    };
    let Some((plic, lines)) = Plic::open(
        PolePie::from_token(plic_pier.token()),
        PolePie::from_token(dtb_pier.token()),
    ) else {
        runtime::env::room::exit_with(13);
    };
    // 三样都得有：一个 S 外部 context（不然影子都投不出去）、控制器自报的线数、以及
    // **设备树里至少一个指向它的中断源**。缺任何一样都是"这台机器的中断面接不上"——
    // 宁可当场收场，也别装作上线了（那样现象是"中断静默不响"，最难查的一类）。
    if plic.contexts.is_empty() || plic.ndev == 0 || lines.count() == 0 {
        runtime::env::room::exit_with(14);
    }
    PLIC.with(|p| *p = Some(plic));
    LINES.with(|l| *l = Some(lines));
    let irq = HolePie::from_token(irq_pier.token());

    // 4. 上线：本域的工作就是下面这个循环。
    //
    // **上线不打日志**：此刻控制台可能还不存在（本域排在它前面起），而第一条投递
    // 一定在它之后（正是它登记了这条线）。故本域只在**第一次投递**时说一句话。

    let mut buf = [0u8; irq::LEN];
    let mut token = [0u8; 1];
    let mut first = true;
    loop {
        // ① 报文：**非阻塞探测**（晚一拍无所谓——登记与写属主都是引导期事件）。
        //
        // 用裸 `pull_from` 而不是 `pull_timeout`：它一并交回**内核盖章的推者**，
        // 而 `Refer` 的判据正是"推者是不是本域的 sire"（见 [`serve`]）。
        if let Ok((n, from)) = mail::pull_from(entry.token(), buf.as_mut_ptr(), buf.len()) {
            serve(&buf[..n], from, sire);
        }
        // ② 中断：有界等待内核的空令牌；有信即醒（`try_push` 唤醒站点）。
        if matches!(irq.wait(HoleDir::Pull, IRQ_WAIT_MS), Ok(true)) {
            // 槽里可能有不止一枚（多 hart 同时取到 SEI 时内核各推一枚）。
            while irq.pull_timeout(&mut token, 0).is_ok() {
                deliver(&mut first);
            }
        }
    }
}

/// 处理一条报文：**认动词、认人**，其余交给线表。
///
/// 认人有两条，都在这里落地：
/// - `Register` 的判据在**行里**（线表判"名字是不是你的"）；
/// - `Refer` 的判据在**血缘**：推者必须是本域的 `sire`（= root 的主线程，生本域的那个
///   任务）。属主只能由 root 写——它不在报文里自证，而是"这枚门闩是谁生的"。
fn serve(msg: &[u8], from: env::TaskId, sire: env::TaskId) {
    let Some(req) = irq::Request::decode(msg) else {
        return;
    };
    match req {
        irq::Request::Register { name, session, ack } => {
            let status =
                match LINES.with(|l| l.as_mut().map(|l| l.register(&name, from, session.get()))) {
                    Some(Ok(line)) => {
                        PLIC.with(|p| {
                            if let Some(p) = p {
                                p.enable(line, LINE_PRIORITY);
                            }
                        });
                        irq::Ack::Ok
                    }
                    // 拒绝：会话门闩那一份本域**当即放下**——它不是我们的资源（不放下就是
                    // 让一个被拒的请求在本域的表里留下痕迹）。行与线都不动。
                    Some(Err(why)) => {
                        let _ = mail::release(session.get());
                        irq::Ack::Refused(why)
                    }
                    None => {
                        let _ = mail::release(session.get());
                        irq::Ack::Refused(irq::Refused::Unknown)
                    }
                };
            reply(ack.get(), status);
        }
        irq::Request::Refer { name, who, ack } => {
            // `who = 0` 是坏报文（行里的 0 是"没人认领"的哨兵，不能被写成属主）。
            let status = if from != sire || who.get() == 0 {
                irq::Ack::Refused(irq::Refused::NotYours)
            } else {
                match LINES.with(|l| l.as_mut().map(|l| l.refer(&name, who))) {
                    Some(Ok(())) => irq::Ack::Ok,
                    Some(Err(why)) => irq::Ack::Refused(why),
                    None => irq::Ack::Refused(irq::Refused::Unknown),
                }
            };
            reply(ack.get(), status);
        }
    }
}

/// 回一条回执，**随即放下本域那一份回执孔**（它是调用方的东西，一个往返就该走完）。
///
/// 调用方可能已经走了（`irq::client` 在等不到回执时封印它自己的回执孔）⇒ 这一推当场
/// 拿到 `Dead`，不会把本域挂住。
fn reply(ack: usize, status: irq::Ack) {
    let _ = HolePie::from_token(ack).push(&[status.byte()]);
    let _ = mail::release(ack);
}

/// 领一条线、投给客户端、结掉它；投不出去就**收线**。
///
/// 三条会静默咬人的规矩都在这里落地（§7.2）：claim 之后**必须真的碰设备**
/// （我们确实在领线号）、`complete` 不是重武装点（所以线还得靠客户端重新等）、
/// 线号 0 恒无（第二个 claim 拿到 0 —— 什么都不做，**不要 complete 0**）。
///
/// 收线的判据是**一次失败**（§12 ②）：`Denied` = 那枚副本已经不在本域表里（客户端死了
/// ——内核沿派生链把本域手里那一枚一起摘掉了）、`Dead` = 客户端封印了它（自愿退场）。
/// `Busy` **到不了这里**：`HolePie::push` 对槽满是等（背压），不是报错。
fn deliver(first: &mut bool) {
    let Some(line) = PLIC.with(|p| p.as_ref().map(|p| p.claim())) else {
        return;
    };
    if line == 0 {
        return; // 没东西可领：不 complete（那会把 0 当线号结掉）
    }
    let client = LINES.with(|l| l.as_ref().and_then(|l| l.holder(line)));
    let mut carried = false;
    if let Some(hole) = client {
        let payload = (line as u16).to_le_bytes();
        carried = HolePie::from_token(hole).push(&payload).is_ok();
    }
    // **先 complete、再收线**（§12 ② 的次序）：领了就结，不然那条线在本域这边静默失联。
    PLIC.with(|p| {
        if let Some(p) = p {
            p.complete(line);
        }
    });
    if client.is_some() && !carried {
        retire(line);
    }
    if carried && *first {
        // **一次性标记**：证明"claim → 投递"整链真的走通过。此后不再打印——
        // 每键一行会把屏幕刷满，而这条链的验证只需要一次。
        *first = false;
        report(line);
    }
}

/// 收线：实例摘空（**行保留**）、关掉那条线、放下本域手里那份会话门闩。
///
/// 行的名字、线号与属主都不动：名字还是 root 写的那个人所有——它换个新实例再来登记，
/// 表里已经有它的位置（这正是"行保留，等 root 重发"，§12 ②）。
fn retire(line: u32) {
    let session = LINES.with(|l| l.as_mut().and_then(|l| l.retire(line)));
    if let Some(session) = session {
        let _ = mail::release(session);
    }
    PLIC.with(|p| {
        if let Some(p) = p {
            p.disable(line);
        }
    });
}

/// 报一行（**只此一次**）：走控制台服务，作普通客户端。
///
/// 本域**不碰设备**（它持的是 PLIC 的寄存器，不是控制台）——一个驱动打印自己，
/// 走的是"客户端 → 控制台服务"这条普通路。
///
/// **长度是设计的一部分**：控制台协议一条消息只带 24 字节（`PAYLOAD_LEN`），更长的
/// 串会被分片、每片一次往返——而两片之间可能插进别的客户端输出（比如服务自己的行
/// 重绘）。故这条标记**恰好 24 字节、一片到底**，在日志里是原子的一行。
///
/// 失败即闭嘴：诊断不该拖住机制（此时若控制台不在，唯一损失是少一行字）。
fn report(line: u32) {
    let Some(session) = console() else {
        return;
    };
    // 手写十进制：本域只用这一行字，拖 `format!` 进来只为它不值（也与 panic 侧的
    // "不能分配"惯例同形）。
    let mut out = [0u8; 24];
    let mut n = 0;
    n += put(&mut out[n..], b"plic: line ");
    n += put_u32(&mut out[n..], line);
    n += put(&mut out[n..], b" delivered\n");
    if let Ok(text) = core::str::from_utf8(&out[..n]) {
        let _ = session.write(text);
    }
}

fn put(dst: &mut [u8], s: &[u8]) -> usize {
    let n = s.len().min(dst.len());
    dst[..n].copy_from_slice(&s[..n]);
    n
}

fn put_u32(dst: &mut [u8], mut v: u32) -> usize {
    let mut buf = [0u8; 10];
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    put(dst, &buf[i..])
}

/// 控制台入口：**只在报那一行时用一次**，用完即弃。
fn console() -> Option<Console> {
    // 目录请求门闩在启动那一小段手里（放进静态，免得把它一路传下来）。
    let entry = ENTRY.with(|e| e.take())?;
    let dir = Directory::open(HolePie::from_token(entry)).ok()?;
    let token = dir.connect_token("console").ok()?;
    Console::open(HolePie::from_token(token)).ok()
}

/// 目录请求门闩（启动时存下；本域只在报那一行时用一次）。
static ENTRY: Lock<Option<usize>> = Lock::new(None);
