#![no_std]
//! protocol — 用户态协议层。
//!
//! 重新开张的条件只有一条：**一个任务要让另一个任务做事**。
//! 在那之前不需要协议——跨域要说的话走 `env` 的调试面（`DebugCall`：内核把固件的调试
//! 控制台直接借给域），一条孔都不用开。`echo` 就是这么说话的。
//!
//! **目前有六份正文**：[`system`]、[`principal`]、[`session`]、[`board`]、[`operator`] 与 [`firmware`]。
//!
//! 落地程度不一样：**四份已经有代码跑在机器上**（[`system`]、[`session`]、[`board`] 与
//! [`operator`]），[`principal`] 只有正文、它的 Server 还没起步。
//!
//! - [`system`] = **服务编排**（systemd 那一层）：系统由哪些 Service 构成、怎么起停监督。
//!   它的载体是内核 ABI（`env::fid` 的 `UnitCall` 整类 + `RoomCall` 的 `Reap`/`Doom`），
//!   那份载体叙述整体降级为该模块的**附录**——载体不等于协议。**它的 Server 是一个独立域**
//!   （`prog-system`）：`system/desk.rs` 是那张服务表，`programs/.../supervisor/system/`
//!   是它落地的那一台。起它的那一枚（引导域 `root`）只做**固件那一层**的事：读 boot 的
//!   两块账、把字节与门闩按单子交出去、退出即停机——它不认识服务名，也不记账。
//! - [`firmware`] = **固件面**：引导域向上层露的那一面——"一张单子换一段记录"（按名发货，
//!   原件与 `VEST` 都留在引导域手里）。两个角色：`server`（引导域的发货循环）、`client`
//!   （编排域去领）。
//! - [`principal`] = **策略身份**："这个请求代表谁"。Server 未落地，但地基已经能看见
//!   （两条不可伪造的身份凭证）。
//! - [`session`] = **会话建立**："两个陌生实体怎么建起一条会话"。身份由内核盖、地址靠
//!   对方交、认领按"谁开的这扇门"——不需要 Server 就能成立，而其余三份都建在它上面。
//! - [`board`] = **命名寻址**："这个名字此刻指向哪个入口"。一块公示板、一枚牌子、
//!   三个动作；判据只有一条（那枚入口是你亲手交给持板者的），**不存预约表**。
//! - [`operator`] = **命名寻址（树那一版）**：一个 Operator 管着所有条目，其他任务只是
//!   操作它——`land` 落 / `part` 分 / `find` 寻 / `trim` 剪（`list` 未上线），落在那棵
//!   `Entry { 名字, 去处 }`、`Node = Pane | Tile` 的树上。核心、载体、服务三层都在
//!   （`prog-operator` 一个域 + `service.rs` 装配里那一格）。与 [`board`] 并存——那是
//!   **另一件事**（公示板 + 待客台账），不是它的旧版。
//!
//! [`session`] 是 `system` 起服务时等就绪的那一步；[`board`] 是 `programs` 里那**一枚**
//! 板线程（招待所有客人，见该模块"板为什么就一枚线程"）+ 装配者域里共享的那一份板
//! （服务怎么问在 [`board::client`]；板那一台与装配侧在 `programs/src/board/`）。
//!
//! [`principal`] 缺的是**地址**：客户端要找到 Principal Server，靠的是 [`board`]。
//! （[`system`] 那一侧的编排者今天不用板查名字——它按静态装配单起服务（`service::PLAN`）、
//! 拿板当"收尸的道"；板是运行期那一步，接在**客户端与服务**之间。）
//!
//! # 地板（可用，不可改）
//!
//! ```text
//! env      ABI：内核与用户态都要的线格式与调用骨架（EnvCall / wire / Permission）
//! runtime  机制：把 ABI 落成可用的运行时
//!            mail   投 / 等（Hole / Pole / Nole；单槽、变长、无 mtu）
//!            tole   组（把几枚可等地挂到一处：孔的一个方向 / 一枚铃）
//!            pie    授出 / 收下（ship / Accord / Reserve / release）
//!            unit   任务与域（build / spawn / hatch / join）
//!            room   域的生死（park / reap / exit / doom）
//!            chrono · memory · heap · lock · dock · bell
//! ```
//!
//! 依赖方向不变：`protocol → runtime → env`（单向）。`kernel/Cargo.toml` 里没有本 crate，
//! 故"内核不知道上层协议"仍是编译期保证。
//!
//! 表里 `runtime` 的 `unit` / `room` 两行（Unit 的生命周期）有一份**更细的正文**：
//! [`system`] 的附录——六个动词、五道门、血缘、两阶段扑杀都在那里，此处不重复。
//!
//! # 这一版**不要**重犯的六条
//!
//! 它们不是风格偏好，是旧树（tag `proto-v1-baseline`）里量出来的读数换的：
//!
//! 1. **一份帧形、一张负码表、一处上界**——五家各写一套，改一处要改五遍。
//! 2. **握手必须两侧同命**：服务侧"先建好会话、等客户端来认领"，客户端一走就留下一个
//!    孤会话；旧树的读数是 `console: session opens=12 ok=12 closed=0 held=1 code=-1`
//!    ——手里有会话，服务侧不认它。
//! 3. **客户端不该有会话账本**：会话的生死归内核的寿命边（开者退场 ⇒ 它开的资源一起封印），
//!    不该由调用方"取走 → 放回"地记账。
//! 4. **判活的探针只能是"对端开的"那一枚孔**——自己开的那一枚判不出对端死活。
//! 5. **一格判据只问一件事**（旧树把"没会话 / 服务不认 / 回信迟到"压成同一个 `0`）。
//! 6. **写同一个设备的人只有一个**，且一次写必须是一条完整的字（旧树里 root 的设备直连写
//!    与服务的写互相插字，把期望串插坏 ⇒ 假红）。

// 码头的泊位是**一张可增长的账**（`session::core::Quay`）：条数由调用方按路数决定，
// 故本 crate 引 `alloc`（与 `env`/`runtime` 同款；备不下时由 `Vec::try_reserve` 如实报
// `Seat::NoRoom`，不 panic）。
extern crate alloc;

pub mod board;
pub mod firmware;
pub mod operator;
pub mod principal;
pub mod session;
pub mod system;

// 依赖先留着：`env` 与 `runtime` 是地板，第一条协议操作出现时立刻要用。
// （本文件暂时没有代码，故 `cargo` 若报 unused dependency，那是预期内的噪音。）
