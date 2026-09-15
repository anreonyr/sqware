//! console — 控制台协议（U/S 共享单一真相）。
//!
//! 控制台是**一个服务**（`prog-console`），不是每程序自己读 UART：终端渲染、
//! 键盘解码、行编辑都住在服务侧，客户端只说"我写了什么"和"给我读一行"。
//!
//! 四块分工：
//!   [`wire`]    —— 线格式：动词 + 变长帧，纯函数；
//!   [`open`]    —— 开会话：握手的**两端**（私有握手孔 + 认领号 + 回信孔的交接）；
//!   [`client`]  —— 线对侧：`Console`/`Readline`（与旧 `programs/src/term` 的 API 同形）；
//!   [`server`]  —— 服务侧：ANSI 渲壳 + VTE 解码 + 行编辑 + 客户端表。
//!
//! **判活与收场不在这里**：那是 [`crate::session`]（五家共用的那一件）。本模块只管
//! "开会话"——那是 console 自己的形状（私有握手孔、认领号、首帧兼开门请求）。
//!
//! # 设备不在这一层，也不在这一域
//!
//! 写由**串口驱动域**（`prog-uart`）落到 UART：客户端 `Write` 只把字节推进**请求
//! 门闩**，本服务渲染成字节后经 [`server::State::take_out`] 交给**请求线程**，
//! 由它按 `protocol::uart` 同步写给驱动（`Ok` 即"已落屏"）。客户端那次 `push`
//! 的阻塞仍是背压——与旧的 `io::put`（同步写设备）语义同级。
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
//!
//! 同一条规矩也管**落屏**：输入线程的每一次重绘只进出帧槽，落屏由请求线程做
//! ——它才持着写设备那枚门闩（[`server::State::take_out`]）。

pub mod client;
pub mod open;
pub mod server;
pub mod wire;

pub use client::{Console, Readline};
pub use server::{Decoder, Key, State, TICK_MS};
pub use wire::{CAP, LINE, Op, ProtocolError, Query, Reply, SERVICE, Text, WORD};

/// 一次往返的上界（毫秒）：**两端同一个数**。
///
/// 客户端等回复用它，服务端推回复也用它——服务比客户端先放弃没有意义（对面还在等），
/// 而后放弃的那一端就是被钉住的那一端：服务只有一个请求线程，它一停，所有客户端一起停。
pub const REPLY_MS: usize = 1000;
