#![no_std]
#![no_main]
//! console — 控制台服务（S 态 supervisor 域）：**只做终端语义**，两线程。
//!
//! ```text
//! 主线程      pull 请求孔 → Open/Write/Close/ReadLine → 出帧落屏 → 回信孔；顺路取整行
//! 输入线程    等投递孔（uart 驱动投来的一拍字节）→ 解码 → 行编辑 → 整行放进共享槽
//! ```
//!
//! # 设备不在本域
//!
//! 设备在 `prog-uart`：它登记那条线、排空 FIFO、把字节投进**投递孔**。本域只做终端语义
//! ——渲染、键盘解码、行编辑、会话表。**写设备是一次跨域协议调用**（`protocol::uart`），
//! 故它只归**主线程**：门闩是 per-task 的，输入线程推不动（与回信孔同一条规矩，踩过）。
//!
//! 输入线程的每一次重绘因此只进**出帧槽**（`State::take_out`），落屏由主线程做——
//! 次序是硬的：`Write` 的回执含义是"这段已落屏"，出帧没送出去就回执等于把承诺降级。
//!
//! # 为什么两个线程
//!
//! 第一版把 `ReadLine` 就地阻塞在请求循环里，于是**等输入期间消息不显示**——别的程序
//! 打印时，屏幕只剩反复重绘的提示符。分成两半之后输出请求随时能插进来，且服务侧会把
//! "正在编辑的那一行"擦掉重画（`protocol::console::server::State::write`）。
//!
//! # 整行为什么经**共享内存**交给主线程，而不是经孔（踩过两遍）
//!
//! **回信孔的 token 只在主线程的 pie 表里**（`Open` 时由客户端交出，主线程登记）。
//! 输入线程是**另一个 task**：拿那个 token 去 `push`，内核在自己的表里找不着 ⇒
//! `Denied` ⇒ 整行永远递不出去。现象极具误导性：**逐键重绘全对**，只有"回车之后什么都
//! 没有"。故整行放进共享态（`set_pending`/`take_pending`），**只有主线程碰孔**。
//!
//! # 空转率（如实记）
//!
//! 有会话等读时，主线程在请求孔上以 [`IDLE_MS`] 为界轮询（顺路取整行）；没有会话等读时
//! 用 [`SLOW_MS`]。**两者都不能是无穷**：整行是被放进共享槽的，主线程得有机会去看。
//! 输入线程只在**有会话等读时**才收字节（没人在读就别把字节从孔里取走——政策留在服务侧，
//! 那是终端语义），没人在读时以 [`TICK_MS`] 为界小睡。
//!
//! # 设备侧那些东西的去处（免得读者以为漏了）
//!
//! 设备视图与 `device_put`、排空循环、`IER` 静音那一对、中断会话门闩、线登记与设备名
//! 的收取——**整批搬进 `prog-uart`**（它才是那条线的持有者）。协议层那个 `Sink` 注入点
//! 随拆域一起消失：渲染不再落设备，只进出帧槽。

extern crate alloc;

// 本包 lib 提供 `_start` + panic_handler；必须真的链接它，`use` 只带符号不算。
extern crate programs;

use core::sync::atomic::{AtomicUsize, Ordering};

use env::TeamId;
use protocol::console::CAP;
use protocol::console::{Decoder, Reply, SERVICE, State, TICK_MS};
use protocol::dispatch::client::Directory;
use protocol::uart::Uart;
use runtime::core::handshake::{self, Pier, Quay};
use runtime::core::lock::Lock;
use runtime::core::port::{Access, Policy, ship};
use runtime::env::mail::HolePie;
use runtime::env::room::sleep;
use runtime::env::task as utask;

/// 找设备服务（`uart` 驱动）的重试次数 × 间隔（毫秒）：**引导期窗口**——它比本域起得早，
/// 但它注册进目录的时机是它自己的事。用尽即收场（见 [`EXIT_MUTE`]）。
const UART_RETRY: usize = 20;
const UART_RETRY_MS: usize = 25;

/// **没有设备服务的控制台没有存在的意义**：写是本域唯一的输出通路，连不上就只是个
/// 吞请求的黑洞。收场之后 root 会按预算重发（它是本域的监护者），新实例再连一次。
const EXIT_MUTE: usize = 20;

/// 主线程的**快档**等待（毫秒）：有会话在等读时用它——输入随时可能来，主线程得及时把
/// 那一行取走并推回会话。
const IDLE_MS: usize = 20;

/// 主线程的**慢档**等待（毫秒）：没人等读时用它。**不能是无穷**（见模块头）。
const SLOW_MS: usize = 200;

/// 投递孔（输入线程用）：本域那份 READ 副本的 token。
///
/// 写成静态是为了"先 `Accord`、再写、最后 `Hatch`"这条次序（`Spawn` 恒产 `Held`）；
/// 与旧版那条中断会话门闩同一个形状，只是孔的另一端从 PLIC 换成了 uart 驱动。
static DELIVER: AtomicUsize = AtomicUsize::new(0);

/// 服务共享状态：请求线程写、输入线程也写（行编辑），故必须互斥。
static CONSOLE: Lock<State> = Lock::new(State::new());

/// 输入线程主体：**等投递孔 → 解码 → 行编辑**，收尾时把整行放进共享槽。
///
/// 没人等读时**不取字节**（取走就丢在没人看的行缓冲里）：以小睡为界等下一次机会。
/// 这是**终端语义**，故它留在本域；设备侧那边只管"有字节就投"，收不下时背压落在
/// 投递孔的槽上（单槽 + `push` 阻塞）。
extern "C" fn input_loop() -> ! {
    // 解码器住在**本线程**的栈上：`Parser` 不是 `Send`，进不了共享态；而它要跨多次
    // 读行存活（转义序列可能被读行边界切开），故放在循环外。
    let mut dec = Decoder::new();
    let hole = HolePie::from_token(DELIVER.load(Ordering::Acquire));
    let mut buf = [0u8; protocol::uart::DELIVER];
    loop {
        if !CONSOLE.with(|s| s.is_reading()) {
            sleep_ticks(TICK_MS);
            continue;
        }
        // 无界等：驱动排空设备后会把这一拍字节投过来（`push` 唤醒站点）。
        let Ok(n) = hole.pull(&mut buf) else {
            continue;
        };
        for &b in &buf[..n] {
            feed(&mut dec, b);
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

/// 睡一拍（`TICK_MS` 的语义：让出处理器，不是"等这么久"）。
fn sleep_ticks(ms: usize) {
    let _ = sleep(core::time::Duration::from_millis(ms as u64));
}

/// 把这一轮攒下的出帧送到设备：**同步**——返回即已落屏（设备侧 `THRE` 轮询完成才回执）。
///
/// 写不出去就是设备服务没了：本域没有第二个出口，**收场**（见 [`EXIT_MUTE`]）。
fn flush(writer: &Uart) {
    let out = CONSOLE.with(|s| s.take_out());
    if !out.is_empty() && writer.write(&out).is_err() {
        runtime::env::room::exit_with(EXIT_MUTE);
    }
}

/// 连设备服务：目录里按名字连（有界重试，见 [`UART_RETRY`]）。
fn connect_uart(dir: &Directory) -> Option<Uart> {
    for _ in 0..UART_RETRY {
        if let Ok(token) = dir.connect_token(protocol::uart::SERVICE) {
            if let Ok(u) = Uart::open(HolePie::from_token(token)) {
                return Some(u);
            }
        }
        sleep_ticks(UART_RETRY_MS);
    }
    None
}

/// 上线时说一句：**"本域拿到了设备服务与投递孔"**。
///
/// 它是这条路的可见证据——`kill console` 之后重发出来的新实例，只有把这两样都拿到手才
/// 走得到这一行。旧版那句 `console: line … ok` 报的是"线登记成功"（那时持线的是本域）；
/// 现在持线的是 `uart`，本域要证明的换成了"我跟它接上了"。长度控制在一条报文以内，
/// 日志里是原子的一行。
fn announced(writer: &Uart) {
    CONSOLE.with(|s| s.banner("console: uart ok\n"));
    flush(writer);
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
    // 2. 自建请求孔——服务自己开自己的门。
    let entry = match HolePie::unseal() {
        Ok(h) => h,
        Err(_) => runtime::env::room::exit_with(3),
    };
    let sire = match utask::sire() {
        Ok(t) => t,
        Err(_) => runtime::env::room::exit_with(4),
    };
    let at_parent = match ship(&down, sire, Access::READ | Access::WRITE, Policy::NONE) {
        Ok(to) => to.seed(),
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
    if dir.register(SERVICE, &entry).is_err() {
        runtime::env::room::exit_with(9);
    }

    // 4. 收**投递孔**：root 开辟并留源副本，本域拿 READ 那一份（uart 驱动拿 WRITE 那份）。
    let deliver_pier = match Pier::pull(&down) {
        Ok(p) => p,
        Err(_) => runtime::env::room::exit_with(10),
    };
    let deliver = HolePie::from_token(deliver_pier.token());

    // 5. 连设备服务：本域唯一的输出通路（连不上即收场——见 [`EXIT_MUTE`]）。
    let Some(writer) = connect_uart(&dir) else {
        runtime::env::room::exit_with(EXIT_MUTE);
    };
    announced(&writer);

    // 6. 输入线程：先产（Held）→ 授投递孔的 READ 副本 → 放行。
    let entry_va = input_loop as extern "C" fn() -> ! as usize;
    let input = match utask::spawn(TeamId(0), entry_va, &[], 0) {
        Ok(t) => t,
        Err(_) => runtime::env::room::exit_with(11),
    };
    let at_input = match ship(&deliver, input, Access::READ, Policy::NONE) {
        Ok(to) => to.seed(),
        Err(_) => runtime::env::room::exit_with(12),
    };
    DELIVER.store(at_input.get(), Ordering::Release);
    if utask::hatch(input).is_err() {
        runtime::env::room::exit_with(13);
    }

    // 7. 请求循环：没人等读时慢档等请求；有人等读时快档，顺路取走输入线程放下的整行。
    let mut req = [0u8; CAP];
    loop {
        let timeout = if CONSOLE.with(|s| s.is_reading()) {
            IDLE_MS
        } else {
            SLOW_MS
        };
        if entry.pull_timeout(&mut req, timeout).is_ok() {
            let outcome = CONSOLE.with(|s| s.serve(&req));
            // 次序：**先落屏、再回执**（`Ok` 的含义是"这段已落屏"）。
            flush(&writer);
            // `ReadLine`：已登记等读，那一行的回执由输入线程放进共享槽（回执为 `None`）。
            if let Some(reply) = outcome.reply {
                route(outcome.to_client, reply);
            }
            continue;
        }
        // 请求孔空着 → 取一次输入线程放下的整行（同一拍只取一次，不做忙等）。
        let pending = CONSOLE.with(|s| s.take_pending());
        flush(&writer);
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
    let (msg, n) = reply.encode();
    if let Some(token) = CONSOLE.with(|s| s.reply_token(client)) {
        let _ = HolePie::from_token(token).push(&msg[..n]);
    }
}
