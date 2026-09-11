#![no_std]
#![no_main]
//! console — 控制台服务（S 态 supervisor 域）：**唯一持 UART 的任务**，两线程。
//!
//! ```text
//! 主线程      pull 请求孔 → Open/Write/Close/ReadLine → 回信孔；顺路取整行
//! 输入线程    读 UART → 解码 → 行编辑 → 把整行放进共享槽 → 主线程推回信孔
//! ```
//!
//! # 设备从哪来
//!
//! **不是内核给的，是 root 授的**：boot 扫设备树造门闩（`Payload::Region`）→ 配对块
//! 交给 root（`docs/driver.md` §3.3.3）→ 本域上线后 root 用 `Accord` 把
//! `serial@10000000` 那一枚授过来（走启动期下行孔的 `Pier`）。本域 `Open` 它，
//! 于是那段物理区出现在**本域的页表**里，此后读写寄存器就是普通 load/store——
//! 全程没有一个 syscall 跟"设备"有关。
//!
//! # 为什么由本域持设备
//!
//! 终端渲染、键盘解码、行编辑都住在这里（`protocol::console::server`），
//! 而它们全都要读写设备——**设备跟着它的使用者走**。此前设备由固件持有、内核转发，
//! 每输出一个字节要穿两次特权边界（§7.3）。
//!
//! # 为什么要两个线程
//!
//! 第一版把 `ReadLine` 就地阻塞在请求循环里，于是**等输入期间消息不显示**——
//! 别的程序打印时，屏幕只剩反复重绘的提示符。分成两半之后输出请求随时能插进来，
//! 且服务侧会把"正在编辑的那一行"擦掉重画（`protocol::console::server::State::write`）。
//!
//! # 整行为什么经**共享内存**交给主线程，而不是经孔（踩过两遍）
//!
//! **回信孔的 token 只在主线程的 pie 表里**（`Open` 时由客户端交出，主线程登记）。
//! 输入线程经 `unit::try_closure` 派生，是**另一个 task**：它拿那个 token 去
//! `push`，内核 `find(token)` 在它自己的表里找不着 ⇒ `Denied` ⇒ 整行永远递不出去
//! ⇒ 客户端阻塞在无上界的 `pull` 上。现象极具误导性：**逐键重绘全对**（那是输入
//! 线程自己在写设备），只有"回车之后什么都没有"。
//!
//! 也曾试过反过来让输入线程**自建**一条事件孔、`Accord` 给主线程后再推——同样被
//! 拒。两个方向都堵在"跨 task 的孔句柄"上，故不再走孔：整行放进共享态
//! （[`State::set_pending`] / [`State::take_pending`]），**只有主线程碰孔**。
//!
//! 共享内存这条路本仓已有先例可用且已实证：`static CONSOLE: Lock<State>` 本来就是
//! 两个线程共写的（行缓冲就在里面），加一个"待交付"格不引入新机制。
//!
//! # 空转率（如实记）
//!
//! 有会话等读时，主线程在请求孔上以 [`IDLE_MS`] 为界轮询（顺路取整行）；没有会话
//! 等读时用 [`SLOW_MS`]。**两者都不能是无穷**：整行是被放进共享槽的，主线程得
//! 有机会去看——阻塞在无上界的等待上，那一行就搁浅了。输入线程以 [`TICK_MS`] 周期
//! 醒来，**但只在有会话等读时才碰设备**（碰了会抢走字节，实测过）。

extern crate alloc;

// 本包 lib 提供 `_start` + panic_handler；必须真的链接它，`use` 只带符号不算。
extern crate programs;

use core::sync::atomic::{AtomicUsize, Ordering};

use env::{HoleDir, Permission, TaskId};
use protocol::console::MSG_LEN;
use protocol::console::{Decoder, Reply, Sink, State, TICK_MS};
use protocol::dispatch::client::Directory;
use runtime::core::handshake::{self, Pier, Quay};
use runtime::core::lock::Lock;
use runtime::core::unit;
use runtime::env::mail::{self, AnyPie as _, HolePie, PolePie};
use runtime::env::room::sleep;

use programs::uart::Uart;

/// 本服务的名字（root 在启动期把它预约给本域）。
const NAME: &str = "console";

/// 中断线驱动的名字（目录里预约给它的名字；线号由本服务登记）。
const PLIC_NAME: &str = "plic";

/// UART 在 PLIC 上的线号（实测：`interrupts = <10>`，§7.1）。
const UART_LINE: u32 = 10;

/// 中断会话孔的单消息字节数：**u16 线号**（`docs/driver.md` §5.3；实测 `ndev = 95`，
/// 一字节够，但"长度是参数不是约定"，给出 u16）。
const LINE_LEN: usize = 2;

/// **等中断的上界**（毫秒）。不是"轮询周期"：有中断时这一等由内核的 `try_push`
/// 唤醒（立即返回），无中断时它只是兜底——没登记成（PLIC 服务不可用）或设备侧
/// 没拉线时，输入不能就此停摆（那时它退化成有界轮询，行为与搬迁前一致）。
const IRQ_WAIT_MS: usize = 20;

/// 找 PLIC 服务的重试次数 × 间隔（毫秒）：**引导期窗口**——它比本域起得早，但它
/// 注册进目录的时机是它自己的事。有界，失败即降级（见 [`IRQ_WAIT_MS`]）。
const PLIC_RETRY: usize = 20;
const PLIC_RETRY_MS: usize = 25;

/// 注册报文的线格式：`[op u8][line u32][priority u32][hole u64][ack u64][name 32]`。
/// 与 `prog-plic` 的那一份**逐字对应**（同仓两个域之间的私有协议）。
const REG_LEN: usize = 1 + 4 + 4 + 8 + 8 + env::NAME_LEN;
const REG_OP: u8 = 1;
const ACK_OK: u8 = 0;

/// 中断会话（输入线程用）：`(门闩 token, 该门闩的合法推者)`。
///
/// 推者 = **PLIC 驱动的 task id**（`HoleMeta.from` 是内核在 `Push` 时盖的章，伪造
/// 不了——§5.3）。先写 `IRQ_FROM` 再发 `IRQ_SESSION`（Release），读者先取
/// `IRQ_SESSION`（Acquire）——故不会读到"有门闩但没有推者"的中间态。
static IRQ_SESSION: AtomicUsize = AtomicUsize::new(0);
static IRQ_FROM: AtomicUsize = AtomicUsize::new(0);

/// 主线程的**快档**等待（毫秒）：有会话在等读时用它——输入线程正在读设备，回车
/// 随时可能来，主线程得及时把那一行取走并推回会话。
const IDLE_MS: usize = 20;

/// 主线程的**慢档**等待（毫秒）：没人等读时用它。**不能是无穷**（见模块头）。
const SLOW_MS: usize = 200;

/// 设备视图（`Open` 之后才有）。
///
/// 进静态是为了 [`device_put`]：**sink 是 `fn` 指针**（`State` 要进 `static`），
/// 够不着栈上的局部量，故设备视图必须住在某个静态里。锁序：`CONSOLE` → `UART`
/// （sink 只在持 `CONSOLE` 时被调；输入线程只单独持 `UART`）——单向，无环。
static UART: Lock<Option<Uart>> = Lock::new(None);

/// 服务写设备的出口（交给协议层当 sink）。
///
/// 设备还没接手时什么都不做——**不是容错**，是"此刻确实没有设备可写"（见
/// `protocol::console::server::no_device`）。
fn device_put(s: &str) {
    UART.with(|u| {
        if let Some(u) = u {
            u.put(s);
        }
    });
}

/// 服务共享状态：请求线程写、输入线程也写（行编辑），故必须互斥。
static CONSOLE: Lock<State> = Lock::new(State::new(device_put as Sink));

/// 输入线程主体：**排空设备 → 等中断 → 解码 → 行编辑**，收尾时把整行放进共享槽。
///
/// 循环就是 `docs/driver.md` §4 的那一行：`drain → unmask → Wait → Pull → mask`。
/// 读它要注意方向：**等的时候门是开的**（中断能进来），一被叫醒就先关门（`mask`），
/// 再回到开头把 FIFO 吃干净。静音在**设备侧**（自己的 `IER`），故 PLIC 侧不需要任何
/// 静音表、deadline 或扫描（§3.2.5）。
///
/// 门闩那一枚是**主线程给的 READ 副本**（[`IRQ_SESSION`]）：门闩是 per-task 的，
/// 输入线程得在自己表里有一枚才能等它——这一枚由持有者（主线程）用 `Accord` 授出。
///
/// 没登记成（PLIC 服务不可用）时，第 ③ 步退化成一次有界睡眠：**行为与搬迁前一致**
/// （有界轮询），只是不再有"中断一到就醒"这一条。降级不写第二份循环。
fn input_loop() -> ! {
    // 解码器住在**本线程**的栈上：`Parser` 不是 `Send`，进不了共享态；而它要跨多次
    // 读行存活（转义序列可能被读行边界切开），故放在循环外。
    let mut dec = Decoder::new();
    loop {
        // ① 有没有会话在等读？没有就关门睡——**不碰设备**（碰了会抢走别人的字节），
        //    门开着也只会让每个字节白走一遍整条链，然后被丢掉。
        if !CONSOLE.with(|s| s.is_reading()) {
            mask_rx();
            let _ = sleep(core::time::Duration::from_millis(TICK_MS as u64));
            continue;
        }
        // ② 排空：中断只告诉我们"有"，不告诉"有多少"——FIFO 里可能攒了好几个字节。
        //    分两块做，**次序是硬要求**：先在设备锁内把字节收进本线程的栈，再在锁外
        //    解码。解码会走回设备（重绘要写设备、还要拿共享态锁），而 `Lock` 不可
        //    重入——持着设备锁去解码就是自己等自己。
        drain_into(&mut dec);
        // ③ 开门 → 睡到"设备有数据" → 关门。
        unmask_rx();
        match IRQ_SESSION.load(Ordering::Acquire) {
            // 登记成了：睡在会话门闩上，PLIC 驱动投来的线号就是这次唤醒。
            0 => sleep_ticks(IRQ_WAIT_MS),
            session => {
                let _ = HolePie::from_token(session).wait(HoleDir::Pull, IRQ_WAIT_MS);
                take_line(session);
            }
        }
        mask_rx();
    }
}

/// 取走一枚线号，并核**来源**：`from` 是内核在 `Push` 时盖的章（伪造不了），
/// 必须是 PLIC 驱动的 task id；线号也必须是本服务登记的那条（§5.3）。
///
/// 校验失败在 debug 档炸出来——release 下这几行只做"把令牌取走"（门闩是单槽的：
/// 不取走，槽就一直满着，内核那边会关闸门等 timer 重开）。线号本身不携带新信息
/// （这条门闩只为 UART 登记），设备状态才是真相，故调用方不消费它的返回值。
fn take_line(session: usize) {
    let mut buf = [0u8; LINE_LEN];
    let Ok((n, from)) = mail::pull_from(session, buf.as_mut_ptr(), buf.len()) else {
        return;
    };
    // 校验只在 debug 档：release 下这一趟的全部作用就是**把令牌取走**（门闩是单槽的，
    // 不取走槽就一直满着，内核那边会关闸门、等 timer 重开）。
    #[cfg(not(debug_assertions))]
    let _ = (n, from);
    #[cfg(debug_assertions)]
    {
        let line = u16::from_le_bytes(buf);
        debug_assert!(
            n == LINE_LEN
                && from.get() == IRQ_FROM.load(Ordering::Relaxed)
                && line as u32 == UART_LINE,
            "console: unexpected interrupt delivery (from {}, {} bytes, line {})",
            from.get(),
            n,
            line
        );
    }
}

/// 睡一拍（`TICK_MS` 的语义：让出处理器，不是"等这么久"）。
fn sleep_ticks(ms: usize) {
    let _ = sleep(core::time::Duration::from_millis(ms as u64));
}

/// 排空设备：字节先落本线程的栈，再在**锁外**解码（见 [`input_loop`] ② 的次序说明）。
fn drain_into(dec: &mut Decoder) {
    /// 一拍最多吃这么多字节（够 16550 的 FIFO 走好几轮；多了就再转一圈）。
    const CHUNK: usize = 64;
    loop {
        let mut buf = [0u8; CHUNK];
        let n = UART.with(|u| {
            let Some(u) = u else {
                return 0;
            };
            let mut n = 0;
            while n < buf.len() {
                match u.try_get() {
                    Some(b) => {
                        buf[n] = b;
                        n += 1;
                    }
                    None => break,
                }
            }
            n
        });
        if n == 0 {
            return;
        }
        for &b in &buf[..n] {
            feed(dec, b);
        }
        if n < CHUNK {
            return;
        }
    }
}

/// 一个字节进解码器；收尾键则把整行放进待交付格（共享态——**只有主线程碰孔**）。
fn feed(dec: &mut Decoder, byte: u8) {
    let Some(key) = dec.advance(byte) else {
        return;
    };
    let done = CONSOLE.with(|s| s.on_key(key));
    if let Some((client, reply)) = done {
        CONSOLE.with(|s| s.set_pending(client, reply));
    }
}

/// 设备侧静音：进门关、出门开（见 [`input_loop`] 的方向说明）。
fn mask_rx() {
    UART.with(|u| {
        if let Some(u) = u {
            u.mask_rx();
        }
    });
}

fn unmask_rx() {
    UART.with(|u| {
        if let Some(u) = u {
            u.unmask_rx();
        }
    });
}

/// 把 UART 的那条线登记给 PLIC 驱动：**会话门闩由本线程造**，READ 副本交给输入
/// 线程、WRITE 副本交给驱动（`docs/driver.md` §3.2.4 的"客户端递出门闩"）。
///
/// 回执走客户端自带的回信门闩——与 dispatch 协议同一条规矩（谁发起谁备回信通道）。
/// 建不成（驱动不在 / 报告被拒）即返回 false：**降级是有界轮询**，不是错误路径
/// （[`input_loop`] 只有一份循环）。
fn register_line(input: TaskId, dir: &Directory, uart: &Uart) -> bool {
    // ① 找驱动：目录里按名字连（它排在 console 之前起，但它注册进目录的时机是它
    //    自己的事——故有界重试）。
    let mut entry = None;
    for _ in 0..PLIC_RETRY {
        if let Ok(t) = dir.connect_token(PLIC_NAME) {
            entry = Some(t);
            break;
        }
        sleep_ticks(PLIC_RETRY_MS);
    }
    let Some(entry) = entry else {
        return false;
    };
    // ② 驱动是谁：门闩的 `owner` 就是开辟它的那个域（服务自开入口门闩，改不掉）。
    let Ok((_, owner)) = mail::reserve(entry) else {
        return false;
    };
    if owner.get() == 0 {
        return false;
    }
    // ③ 两枚孔：中断会话（驱动投线号）+ 回执（驱动回状态）。
    let Ok(session) = HolePie::unseal(LINE_LEN) else {
        return false;
    };
    let Ok(at_input) = session.accord(input, Permission::READ) else {
        return false;
    };
    let Ok(at_plic) = session.accord(owner, Permission::WRITE) else {
        return false;
    };
    let Ok(ack) = HolePie::unseal(1) else {
        return false;
    };
    let Ok(ack_at_plic) = ack.accord(owner, Permission::WRITE) else {
        return false;
    };
    // ④ 报文：`[op][line][priority][hole][ack][name]`。
    let mut msg = [0u8; REG_LEN];
    msg[0] = REG_OP;
    msg[1..5].copy_from_slice(&UART_LINE.to_le_bytes());
    msg[5..9].copy_from_slice(&1u32.to_le_bytes()); // priority ≥ 1（0 = 静音）
    msg[9..17].copy_from_slice(&at_plic.get().to_le_bytes());
    msg[17..25].copy_from_slice(&ack_at_plic.get().to_le_bytes());
    let name = NAME.as_bytes();
    msg[25..25 + name.len()].copy_from_slice(name);
    if HolePie::from_token(entry).push(&msg).is_err() {
        return false;
    }
    // ⑤ 等回执（有界：注册是同步的，1s 足够）。
    let mut status = [1u8; 1];
    if !matches!(ack.pull_timeout(&mut status, 1000), Ok(1)) || status[0] != ACK_OK {
        return false;
    }
    // ⑥ 交给输入线程：**先写推者、后发门闩**（见 [`IRQ_SESSION`] 的顺序说明）。
    IRQ_FROM.store(owner.get(), Ordering::Relaxed);
    IRQ_SESSION.store(at_input.get(), Ordering::Release);
    let _ = uart; // 设备视图不必传（输入线程从静态取）——此参数只作"确实持着设备"的凭据
    true
}

#[unsafe(no_mangle)]
extern "C" fn main() -> ! {
    // 1. 靠泊 + 自建控制孔（只给父域；与请求孔分离——父域拿不到请求队列）。
    let up = match handshake::moor() {
        Ok(u) => u,
        Err(_) => runtime::env::room::exit_with(1),
    };
    let down = match HolePie::unseal(handshake::MTU) {
        Ok(h) => h,
        Err(_) => runtime::env::room::exit_with(2),
    };
    // 2. 自建请求孔——服务自己开自己的门。
    let entry = match HolePie::unseal(MSG_LEN) {
        Ok(h) => h,
        Err(_) => runtime::env::room::exit_with(3),
    };
    let sire = match runtime::env::task::sire() {
        Ok(t) => t,
        Err(_) => runtime::env::room::exit_with(4),
    };
    let at_parent = match down.accord(sire, Permission::READ | Permission::WRITE) {
        Ok(t) => t,
        Err(_) => runtime::env::room::exit_with(5),
    };
    if Quay::new(at_parent).push(&up).is_err() {
        runtime::env::room::exit_with(6);
    }

    // 3. 收配给：目录请求门闩（dir 亲授，root 只转达）。
    let pier = match Pier::pull(&down) {
        Ok(p) => p,
        Err(_) => runtime::env::room::exit_with(7),
    };
    let dir = match Directory::open(HolePie::from_token(pier.token())) {
        Ok(d) => d,
        Err(_) => runtime::env::room::exit_with(8),
    };
    if dir.register(NAME, &entry).is_err() {
        runtime::env::room::exit_with(9);
    }

    // 3.5 收设备：root 授过来的 UART 门闩（boot → root → 本域，**同一枚**）。
    //     `Open` 之后那段物理区就在本域的页表里了——设备到手，内核不在路上。
    let dev = match Pier::pull(&down) {
        Ok(p) => p,
        Err(_) => runtime::env::room::exit_with(11),
    };
    let uart = match Uart::open(PolePie::from_token(dev.token())) {
        Ok(u) => u,
        Err(_) => runtime::env::room::exit_with(12),
    };
    UART.with(|u| *u = Some(uart));

    // 4. 输入线程：不接任何句柄**直接开工**——它要的那枚中断会话门闩由本线程造好后
    //    经 `Accord` 授给它（门闩是 per-task 的，这是唯一的交接方式）。
    let input = match unit::try_closure(input_loop) {
        Ok(j) => j.id(),
        Err(_) => runtime::env::room::exit_with(10),
    };

    // 4.5 接中断：把设备那条线登记给 PLIC 驱动。**失败不是错误路径**——输入线程的
    //     循环只有一份，没登记成就退化成有界轮询（与搬迁前一致）。
    register_line(input, &dir, &uart);

    // 5. 请求循环：没人等读时慢档等请求；有人等读时快档，顺路取走输入线程放下的整行。
    let mut req = [0u8; MSG_LEN];
    loop {
        let timeout = if CONSOLE.with(|s| s.is_reading()) {
            IDLE_MS
        } else {
            SLOW_MS
        };
        if entry.pull_timeout(&mut req, timeout).is_ok() {
            let outcome = CONSOLE.with(|s| s.serve(&req));
            // `ReadLine`：已登记等读，那一行的回执由输入线程放进共享槽（回执为 `None`）。
            if let Some(reply) = outcome.reply {
                route(outcome.to_client, reply);
            }
            continue;
        }
        // 请求孔空着 → 取一次输入线程放下的整行（同一拍只取一次，不做忙等）。
        let pending = CONSOLE.with(|s| s.take_pending());
        if let Some((client, reply)) = pending {
            route(Some(client), reply);
        }
    }
}

/// 把一条回复推到某会话的回信孔。
///
/// **必须在主线程调**：`token` 取自 `State` 的会话表，而那是**本线程**表里的号。
fn route(to_client: Option<usize>, reply: Reply) {
    let Some(client) = to_client else {
        return;
    };
    let msg = reply.encode();
    if let Some(token) = CONSOLE.with(|s| s.reply_token(client)) {
        let _ = HolePie::from_token(token).push(&msg);
    }
}
