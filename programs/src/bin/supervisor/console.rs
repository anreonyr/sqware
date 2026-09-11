#![no_std]
#![no_main]
//! console — 控制台服务（S 态 supervisor 域）：**唯一读 UART 的任务**，两线程。
//!
//! ```text
//! 主线程      pull 请求孔 → Open/Write/Close/ReadLine → 回信孔；顺路取整行
//! 输入线程    读 UART → 解码 → 行编辑 → 把整行放进共享槽 → 主线程推回信孔
//! ```
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

use env::Permission;
use protocol::console::MSG_LEN;
use protocol::console::{Decoder, Reply, State, TICK_MS};
use protocol::dispatch::client::Directory;
use runtime::core::handshake::{self, Pier, Quay};
use runtime::core::lock::Lock;
use runtime::core::unit;
use runtime::env::mail::{AnyPie as _, HolePie};
use runtime::env::room::sleep;

/// 本服务的名字（root 在启动期把它预约给本域）。
const NAME: &str = "console";

/// 主线程的**快档**等待（毫秒）：有会话在等读时用它——输入线程正在读设备，回车
/// 随时可能来，主线程得及时把那一行取走并推回会话。
const IDLE_MS: usize = 20;

/// 主线程的**慢档**等待（毫秒）：没人等读时用它。**不能是无穷**（见模块头）。
const SLOW_MS: usize = 200;

/// 服务共享状态：请求线程写、输入线程也写（行编辑），故必须互斥。
static CONSOLE: Lock<State> = Lock::new(State::new());

/// 输入线程主体：等读期间读设备、做行编辑，收尾时把整行放进共享槽。
///
/// 全程**不碰任何门闩**——它只读设备、改共享态。设备是"环境"给的权限
/// （`IOCall`），不占权限表，故这条线程不需要任何交接句柄。
fn input_loop() -> ! {
    // 解码器住在**本线程**的栈上：`Parser` 不是 `Send`，进不了共享态；而它要跨多次
    // 读行存活（转义序列可能被读行边界切开），故放在循环外。
    let mut dec = Decoder::new();
    loop {
        // ① 有没有会话在等读？没有就睡——**不碰设备**（碰了会抢走别人的字节）。
        if !CONSOLE.with(|s| s.is_reading()) {
            let _ = sleep(core::time::Duration::from_millis(TICK_MS as u64));
            continue;
        }
        // ② 设备有字节就处理一个（**每轮只持锁一拍**，输出请求随时能插进来）。
        let Some(byte) = runtime::env::io::try_get() else {
            let _ = sleep(core::time::Duration::from_millis(TICK_MS as u64));
            continue;
        };
        let Some(key) = dec.advance(byte) else {
            continue;
        };
        // ③ 落到共享状态；若这一键收尾，把「哪个会话 + 什么结果」放进待交付格。
        let done = CONSOLE.with(|s| s.on_key(key));
        if let Some((client, reply)) = done {
            CONSOLE.with(|s| s.set_pending(client, reply));
        }
    }
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

    // 4. 输入线程：只碰共享态与设备，不接任何句柄（故无需交接次序）。
    if unit::try_closure(input_loop).is_err() {
        runtime::env::room::exit_with(10);
    }

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
