//! console — 控制台协议（U/S 共享单一真相）。
//!
//! 控制台是**一个服务**（`prog-console`），不是每程序自己读 UART：终端渲染、
//! 键盘解码、行编辑都住在服务侧，客户端只说"我写了什么"和"给我读一行"。
//!
//! 三块分工：
//!   [`wire`]   —— 线格式：动词 + 定长消息，纯函数；
//!   [`client`] —— 线对侧：`Console`/`Readline`（与旧 `programs/src/term` 的 API 同形）；
//!   [`server`] —— 服务侧：ANSI 渲壳 + VTE 解码 + 行编辑 + 客户端表。
//!
//! # 服务自己持设备
//!
//! 写由**服务**落到 UART：客户端 `Write` 只把字节推进**请求门闩**，服务收到即写设备。
//! 客户端那次 `push` 的阻塞就是背压——与旧的 `io::put`（同步写设备）语义同级。
//!
//! # 一条孔：回信
//!
//! 客户端只开**一条**孔（回信），`Open` 时把它在**服务侧**的 token 交出去；此后
//! 请求走请求门闩、回复走这条孔。第一版还开过第二条"数据孔"——**从第一天就是死码**
//! （客户端从不推、服务从不读），已删。
//!
//! # 服务侧两线程
//!
//! `ReadLine` 在服务侧**只登记不阻塞**，整行由输入线程在回车时交付——否则别的程序
//! 打印的消息会排在请求孔里显示不出来。交付**不能**由输入线程直接推回信孔：那枚
//! token 只在请求线程的 pie 表里（句柄是 per-task 的），故整行经**共享态**交接
//! （[`server::State::set_pending`] → [`server::State::take_pending`]）。

pub mod client;
pub mod server;
pub mod wire;

pub use client::{Console, Readline};
pub use server::{Decoder, Key, State, TICK_MS};
pub use wire::{
    CLIENT_AT, LINE_MAX, MSG_LEN, Op, PAYLOAD_LEN, ProtocolError, REPLY_PEER_AT, Reply,
    Request,
};
