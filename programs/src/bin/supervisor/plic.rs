#![no_std]
#![no_main]
//! plic — 中断线驱动（S 态 supervisor 域，**一个线程**）。
//!
//! ```text
//! 循环     注册请求（探测）→ irq 门闩（有界等待）→ claim → 投线号 → complete
//! ```
//!
//! # 它认识什么、不认识什么
//!
//! 它认识 PLIC 的寄存器布局与设备树的绑定；它**不认识任何设备**——不知道线 10 后面
//! 是串口还是网卡，只把"哪条线响了"投给当初登记那条线的客户端（`docs/driver.md`
//! §3.2.6：线号 → (客户端, 门闩) 的表活在**本域的内存**里，内核不参与）。
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
//! 为这件事踩过坑并写进了它的模块头）。注册报文带来的门闩落在收报文的任务表里，
//! 故收报文与投递必须同任务。于是循环里两件事共处：注册用**非阻塞探测**（它是
//! 引导期事件，晚 20 ms 无所谓），中断用**有界等待**（无界会把注册饿死）。
//! 中断路径本身不轮询：内核的 `try_push` 会唤醒站点。
//!
//! # 线号从哪来
//!
//! 设备树。`interrupts-extended` 的**项序即 context 序**（RISC-V 的中断控制器
//! 绑定），而 `cell == 9` 是 S 模式外部中断、`11` 是 M 模式——**认 9 不认 11**
//! （认错就是把线交给固件）。**不硬算 `2h+1`**（`docs/driver.md` §7.2）。

extern crate alloc;
// 本包 lib 提供 `_start` + panic_handler；必须真的链接它，`use` 只带符号不算。
extern crate programs;

use alloc::vec::Vec;

use env::HoleDir;
use env::Permission;
use protocol::console::Console;
use protocol::dispatch::client::Directory;
use runtime::core::handshake::{self, Pier, Quay};
use runtime::core::lock::Lock;
use runtime::env::mail::{AnyPie as _, HolePie, PolePie};

/// 本服务的名字（root 在启动期把它预约给本域）。
const NAME: &str = "plic";

/// 注册报文的字节数：`[op u8][line u32][priority u32][hole u64][ack u64][name 32]`。
const REG_LEN: usize = 1 + 4 + 4 + 8 + 8 + env::NAME_LEN;

/// 请求孔的单消息上限（与 `REG_LEN` 同量级，给足余量）。
const REG_MTU: usize = 64;

/// `irq` 门闩的等待上界（毫秒）——**不能是无穷**：注册报文要有人听（见模块头）。
/// 中断路径不受它影响（槽满/有信都立即唤醒）。
const IRQ_WAIT_MS: usize = 20;

/// S 模式外部中断的中断号（`interrupts-extended` 里的 cell 值）。
const EXT_S: u32 = 9;

/// 注册成功 / 拒绝。
const ACK_OK: u8 = 0;
const ACK_DENIED: u8 = 1;

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
    /// 开闩 + 读设备树：把"我是谁、我有哪些 context"问清楚。
    ///
    /// 判据 = `interrupt-controller` 且 `compatible` 里含 `plic` 的节点。**解释设备
    /// 树是驱动的事**（内核只原样搬运自描述，§3.1.3），故这里可以按 compatible 认。
    fn open(pole: PolePie, dtb: PolePie) -> Option<Self> {
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
        Some(Self {
            base,
            ndev,
            contexts,
        })
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

/// 一条线的客户端：会话门闩在**本域侧**的 token（往它投线号）。
///
/// 只存 token 值：`HolePie` 没有 `Drop`，"即建即弃"与持有等价（console 协议的
/// `Slot` 同款），而 token 是 `Copy`。
#[derive(Clone, Copy)]
struct Client {
    line: u32,
    hole: usize,
}

/// 注册表：线号 → 客户端。**只在本域的内存里**（§3.2.6）。
static TABLE: Lock<Vec<Client>> = Lock::new(Vec::new());

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
    // 自建请求孔：客户端往它推注册报文。
    let entry = match HolePie::unseal(REG_MTU) {
        Ok(h) => h,
        Err(_) => runtime::env::room::exit_with(8),
    };
    if dir.register(NAME, &entry).is_err() {
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
    let Some(plic) = Plic::open(
        PolePie::from_token(plic_pier.token()),
        PolePie::from_token(dtb_pier.token()),
    ) else {
        runtime::env::room::exit_with(13);
    };
    // 一个 context 都没数出来 = 这台机器的中断面接不上：宁可当场收场，也别装作
    // 上线了（那样现象是"中断静默不响"，最难查的一类）。
    if plic.contexts.is_empty() || plic.ndev == 0 {
        runtime::env::room::exit_with(14);
    }
    PLIC.with(|p| *p = Some(plic));
    let irq = HolePie::from_token(irq_pier.token());

    // 4. 上线：本域的工作就是下面这个循环。
    //
    // **上线不打日志**：此刻控制台可能还不存在（本域排在它前面起），而第一条投递
    // 一定在它之后（正是它登记了这条线）。故本域只在**第一次投递**时说一句话。

    let mut buf = [0u8; REG_MTU];
    let mut token = [0u8; 1];
    let mut first = true;
    loop {
        // ① 注册：非阻塞探测（晚一拍无所谓——它是引导期事件）。
        if let Ok(n) = entry.pull_timeout(&mut buf, 0) {
            serve_register(&buf[..n]);
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

/// 处理一条注册报文：记账 + 使能该线 + 回执。
fn serve_register(msg: &[u8]) {
    if msg.len() < REG_LEN || msg[0] != 1 {
        return;
    }
    let line = u32::from_le_bytes(msg[1..5].try_into().unwrap_or([0; 4]));
    let priority = u32::from_le_bytes(msg[5..9].try_into().unwrap_or([0; 4]));
    let hole = usize::from_le_bytes(msg[9..17].try_into().unwrap_or([0; 8]));
    let ack = usize::from_le_bytes(msg[17..25].try_into().unwrap_or([0; 8]));
    let plic = PLIC.with(|p| p.as_ref().map(|p| (p.ndev, p.contexts.len())));
    // 拒绝的两条：线号越界（本控制器没有这条线）、本域没数出任何 context（连不上）。
    let ok = match plic {
        Some((ndev, ctxs)) if line >= 1 && line <= ndev && ctxs > 0 && hole != 0 => {
            PLIC.with(|p| {
                if let Some(p) = p {
                    p.enable(line, priority.max(1)); // priority 0 = 静音（可逆），故至少 1
                }
            });
            let c = Client { line, hole };
            TABLE.with(|t| {
                t.retain(|x| x.line != line);
                t.push(c);
            });
            true
        }
        _ => false,
    };
    // 回执（客户端自带回信通道——与 dispatch 协议同一条规矩）。
    let status = [if ok { ACK_OK } else { ACK_DENIED }];
    let _ = HolePie::from_token(ack).push(&status);
}

/// 领一条线、投给客户端、结掉它。
///
/// 三条会静默咬人的规矩都在这里落地（§7.2）：claim 之后**必须真的碰设备**
/// （我们确实在领线号）、`complete` 不是重武装点（所以线还得靠客户端重新等）、
/// 线号 0 恒无（第二个 claim 拿到 0 —— 什么都不做，**不要 complete 0**）。
fn deliver(first: &mut bool) {
    let Some((line, client)) = PLIC.with(|p| p.as_ref().map(|p| p.claim())).map(|line| {
        let client = TABLE.with(|t| t.iter().find(|c| c.line == line).map(|c| c.hole));
        (line, client)
    }) else {
        return;
    };
    if line == 0 {
        return; // 没东西可领：不 complete（那会把 0 当线号结掉）
    }
    if let Some(hole) = client {
        let payload = (line as u16).to_le_bytes();
        let _ = HolePie::from_token(hole).push(&payload);
        if *first {
            // **一次性标记**：证明"claim → 投递"整链真的走通过。此后不再打印——
            // 每键一行会把屏幕刷满，而这条链的验证只需要一次。
            *first = false;
            report(line);
        }
    }
    PLIC.with(|p| {
        if let Some(p) = p {
            p.complete(line);
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
