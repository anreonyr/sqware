#![no_std]
//! protocol — **协议那一棵树**：一句话本身（正文 · 形 · 据 · 账）＋ 碰内核的那几件（开口）。
//!
//! 一句话记：**协议是话，程序是说话的人**——域入口、那一台、装配表、需求单、死法、装配次序
//! 都住 `programs`。
//!
//! # 照实记：这一版把「约」并回来了
//!
//! 原有两个 crate：`crates/contract`（「约」，7,876 行：正文 / 形 / 据 / 账 / 不碰内核的适配）
//! 与 `crates/protocol`（「口」，2,004 行：客侧那几手、会话那几手、绑内核的构造）。切法只有
//! 一条：**碰不碰 `runtime`**；`protocol` 里满地的 `pub use crate::…` 就是那条边的形状。
//!
//! **照实记（这条边界为什么退成约定）**：它唯一能被机械检查的消费者是一台**编外宿主靶**
//! （`protocol-case`：`#[path]` 真依赖当时那份会话核心的源码（`session/core.rs`，本刀已删）、由运行时那层给一个
//! 桩），而"约定住哪"那三件（`frame` / `id` / `fail_codes`）当初就是为它单开的文件。那台靶
//! 已删（用户裁定"protocol-case 没必要"，提交 `d0c7166`）⇒ 这条边界**实践价值为零**，本轮
//! 并回一个 crate。
//!
//! **代价照实记**：本 crate 拖 `runtime`（riscv 内联汇编、无 `cfg` 护栏）⇒ **永久不能宿主
//! 链接测试**（见 `Cargo.toml` 那一段）。故"纯核不碰 runtime"此后是一条**纪律，不是编译期
//! 保证**：判定与账仍单独成份（`core.rs` / `frame.rs`），落内核的适配一律另开一份
//! （`client.rs` / [`communication`]）。
//!
//! # 划界只有一条判据（原「约」的正文，判据未改）
//!
//! > **判定与账不碰 `runtime` 那一层。**
//!
//! 这条纪律从前写在**六份文件的头注**里（`system/board/core.rs`、`system/operator/core.rs`、
//! `session/core.rs`（这一份已随会话那一刀删掉）、`driver/line/core.rs`、
//! `programs/.../board/desk.rs`、`programs/.../operator/desk.rs`）——**有纪律，没有边界**。
//! 并回来之后它仍是这一层内部的分家依据：`core.rs` / `frame.rs` 只判、只记；碰内核的住
//! `client.rs` 与 [`communication`] 那几手。
//!
//! # 面上有什么
//!
//! ```text
//!   frame         形：principal 与 coalition **同形的那一份**骨架
//!   id            号：`Id` 那一族怎么编、怎么读
//!   message       报：一族会编会解的那条约定（`store` / `fetch`）
//!   fail_codes    负码表：`fail_codes!` 宏 ＋ 全协议共用的那一格 `OK`
//!   communication 建：一段关系怎么建立、报怎么收发（`establish` / `sender` / `receiver`）
//!                 ——**其余每一份都建在它上面**
//! ```
//!
//! **照实记（`frame` / `id` / `fail_codes` 为什么是一组）**：`frame` 要 `id` 与 `fail_codes`
//! ⇒ 三件是一组，只搬一件编不过。
//!
//! # 照实记（"面会长成什么样"那一节已撤）
//!
//! 它记的是两个 crate（「约」/「口」）分家时对本 crate 的打算：哪几份留下、哪几份搬走。
//! 那一版已经并回一个 crate（见上面的照实记），故这节连同"搬它们那一批时按手劈"的说法
//! 一起撤掉。它唯二还成立的两条落到别处：
//!
//! - `system/{board,operator}/mod.rs` **不碰 `runtime`**，但它 `pub use` 的三手
//!   （`marked_as` / `opened_by` / `vested_by`）直接叫 `mail::reserve`
//!   ——**是内核读**（那支 `reserve_reads!` 宏已随残枝退场，三具身体住 `establish.rs` 本体）。这三手的身体如今只住 [`communication::establish`]（一处分身，谁要谁直接叫；
//!   "两处取名"那一层别名也已经撤了）；
//! - **客侧那几手与碰内核的那几手另开一份**（`client.rs` / [`communication`]），与判定、账
//!   分开摆——这是本 crate 内部的分家依据，与 crate 边界不是一回事。
//!
//! # 这一层装什么：**两句**
//!
//! 1. **给别的 task 用的一切** —— 正文、判定、帧、**客侧那几手**（`*/client.rs`）：从外面找上
//!    某份协议的人**只需要这一面**；
//! 2. **能在宿主上跑的实现** —— 只判、只记、不发消息的那几份（`operator` 的 `judge` / `gate` /
//!    `ledger` 是今天的全部）：它们**不是**给别的 task 用的，是**实现方**的东西。
//!
//! **照实记（第二句是补上的那一半）**：原先只写了第 1 句。第 2 句管的是**依赖面**那条判据
//! ——"别的 task 不该依赖实现"——而它今天**事实上成立**（那三份的读者只有持树者一处，
//! `grep` 得出来），差的是**文件同住**（它们与客侧那几手在同一个模块树里）。
//!
//! **照实记（这一节随宿主靶删掉的那一半，用户裁定"protocol-case 没必要，删了"）**：第 2 句原先
//! 还写着"住在这里的理由是**进得了宿主靶**"——那台编外靶已删，这条理由随之撤；原来跟着它那句
//! "要把文件也分开得先找一处既能被宿主靶逐字编、又算实现的落脚"也就没有前提了，一并删。
//!
//! 与 `programs/src/lib.rs` 的"**判据是谁在说话**"同一条线：这一层是那条线的**接口那一边**，
//! `programs` 是**实现那一边**（实现方跟着"用它那个程序所在的档"走）。
//!
//! **顶层只有四份正文**——这里就是"目录层次"那句话：
//!
//! ```text
//!   communication    地板 ＋ 会话    一枚孔 / 一条路怎么建起来（其余每一份都建在它上面）
//!   system           系统    编排（grant：配给记录的解码）＋ 运行期命名
//!                     └ 容纳  board      运行期的公示板 ＋ 待客账
//!                             operator   命名寻址：一棵树，名字 → Pie
//!                             principal  策略身份：这个 Task 此刻代表谁、从谁而来
//!                             coalition  策略结盟：身份的横向那半（principal 的客人）
//!                             supply     配给：一张单子换一段记录（引导域 ↔ 编排域）
//!   driver           轴      线（line）——**本层只剩这一件**（见下）
//!   （根上三件共享件：`frame` 帧骨架 · `id` 号的规则 · `fail_codes` 负码表）
//! ```
//!
//! **照实记（这一层的边界在残枝那一刀收窄了）**：`system::core`（四条判定）、
//! `system::desk`（服务表 ＋ 待客账）、`driver::supply`（→ `system::supply`）、
//! `driver::line::core`（→ 路由者自己的 `core/lines.rs`）都已搬出——判据是
//! **"只有实现方读得到它"**。`protocol::driver` 因此只剩 [`driver::DIR`] ＋ [`driver::line`]。
//!
//! **`frame.rs` 只在"帧那一半要能被单独编"时才单开**。`driver::line` 的帧整份编得动
//! ⇒ 没有分家的需要。
//! **照实记（`call.rs` 那一格已经收掉）**：
//! 系统那四份 `system/*/call.rs` 是**薄封装**（文件里除 `pub use` 外没有一个自己的 `fn`），已并进
//! 各自的 `mod.rs` ⇒ **协议树上不再有 `call.rs`**。实现树上最后一个也走了：`programs/src/system/call.rs`
//! （编排者的适配）**唯一读者就是 `system/server.rs`**，故并进那个文件。驱动那一侧也走了：
//! `programs/src/driver/rtc/call.rs` 拆进 `rtc/core/`（形与记号归 `core/frame.rs`）⇒
//! 这个名字今天**一处都不剩**：底座那几件手的身体并进了 `communication/establish.rs`
//! （照实记见那一份的文件头）。
//!
//! **照实记（"容纳"是用户裁的）**：`board` 一直在 [`system`] 之下；`operator` / `principal` /
//! `coalition` 原先是**顶层**（与 `system` 平级），裁定之后收进去。**判据是"谁住编排域"**：
//! iii 之后这四套协议的落地（板线程 / 持树者 / 名册 / 盟册）都是**编排域里的线程**，
//! 而 [`system`] 正文自己写着"**要找服务得先有目录**——今天那本目录就是 `board`"。
//! 故**协议树与实现树（`programs/src/system/`）同形**。
//! **被否的那条读法**是"协议树按'谁在说话'分、不该镜像实现树"（我原先的建议）——用户裁的是前者。
//!
//! 落地程度不一样：**三份顶层 ＋ 容纳的四套都已经有代码跑在机器上**（[`system`]、[`driver`]
//! （**只剩 `line` 一件**：客侧四手 ＋ 形与码——账与四原语在持有者那一侧，见 [`driver::line`]，
//! `router` / `uart` / `rtc` 与两位客人 `lodger` / `sleeper` 都跑在机器上）、[`communication`]、
//! [`system::operator`]、[`system::principal`]（**名册 + 谱系**：九条原语、一位真客人
//! `subject`）与 [`system::coalition`]（**横向盟籍**：一张两列表 + 一枚计数器、六条原语、
//! 一位真客人 `member`））。
//!
//! - [`system`] = **服务编排**（systemd 那一层）：系统由哪些 Service 构成、怎么起停监督。
//!   它**有两半**：**编排**（`core` / `desk` / `grant`）与**运行期命名**（[`system::board`]：
//!   "这个名字此刻指向哪个入口"——一块公示板、一枚牌子、**四个动作**
//!   （`REGISTER` / `UNREGISTER` / `LOOKUP` / `EVICT`），判据只有一条：那枚入口是你亲手交给
//!   持板者的，**不存预约表**）。照实记：这一句原写"三个动作"，`EVICT` 上了线之后没收。
//!   两半同住一份正文，因为**板是编排域自己的一个机构**（它那本客人账就是监督的输入；
//!   它由本域起、住本域里）——**照实记（这一句原先举错了另一半）**：原写"名字对上入口的静态
//!   那半是 `grant`"，而 `grant` 管的是**配给记录的解码**（固件回的那段"坐标 + 号"），与名字
//!   无关；装配期那一步在**装配机器**手里（单子上的 `name` ＋ 起手时交出去的入口）。
//!   它的载体是内核 ABI（`env::fid` 的 `UnitCall` 整类 + `RoomCall` 的 `Reap`/`Doom`），
//!   那份载体叙述整体降级为该模块的**附录**——载体不等于协议。**它的 Server 是一个独立域**
//!   （`prog-system`）：服务表与判定已回实现侧（`programs/src/system/{desk,core}.rs`），
//!   `programs/.../system/` 是它落地的那一台。起它的那一枚（引导域 `root`）只做**固件那一层**
//!   的事：读 boot 的两块账、把字节与门闩按单子交出去、退出即停机——它不认识服务名，也不记账。
//! - [`driver`] = **设备轴**：一台设备从"交到某个域手里"到"它的线有人领"这一整段。
//!   本层只剩**线**（权威 / 属主 / 登记 / 投递 / 排空 / 收线——**四格都落在机器上**，
//!   见 [`driver::line`]）：**物料**那一半是配给（[`system::supply`]，递单的是编排域、
//!   发货的是引导域，驱动只是收方），**线那本账**住路由者自己那一侧。
//!   它不是驱动框架：设备语义各驱动自带（`programs/src/driver/<域>/`），装配样板住程序侧
//!   （`programs/src/driver/{assemble,device,context}.rs`），本层只管**跨域约定**。
//! - [`system::principal`] = **策略身份**：**名册**（TID → 此刻代表的 PrincipalId）与**谱系**
//!   （PrincipalId 的一棵只增不改的树）。两条轴都不定义权限——收到它的服务自己解释那条号。
//!   载体建在会话之上：门牌落在树上 `/sys/principal`，一问一答替这一趟借一枚回信孔过去。
//! - [`system::coalition`] = **策略结盟**（策略身份那一条轴的**横向**那半）：**一张两列表**——哪条身份
//!   在哪些盟里、哪枚盟里有谁，反着念是同一个关系的两个方向。号由服务铸（铸过就一直在），
//!   盟无主（故失败域里没有 `Denied`），不产生 PrincipalId、不发 Pie、不解释成员资格的含义。
//!   六条原语（`found` / `enter` / `leave` / `amid` / `band` / `bloc`）——**六条都在线上**
//!   （`frame.rs` 那一排码 `1..6`，BAND / BLOC 那两条已由服务实现）。
//!   照实记：这一句原来写的是"四条上线、两条住核心"，那是 `band` / `bloc` 还没接上时的口径。
//!   它是**身份服务的客人**：每条写原语嵌一次 `Resolve(发送者)`——"self"因此在适配层，
//!   不在核心（正文的"已知边界"里写着这一条的确切含义）。
//! - [`communication`] = **建立与收发**："两个陌生实体怎么建起一条会话"——身份由内核盖、地址靠
//!   对方交、认领按"谁开的这扇门"。**三格动词**（`endpoint` 两头都要 / `give` 只把读端交出去 /
//!   `find` · `claim` 只认对方那一枚），加上两个方向的手柄（[`communication::Sender`] /
//!   [`communication::Receiver`]）。不需要 Server 就能成立，其余每一份都建在它上面。
//! - [`system::operator`] = **命名寻址（树那一版）**：一个 Operator 管着所有条目，其他任务只是
//!   操作它——**对外七条**：`land` 落 / `part` 分 / `find` 寻 / `trim` 剪 / `list` 列 /
//!   `seek` 译 / `name` 名（外加核心那一条只读 `opens`，不上线）。树是**一张按号排的表**
//!   （号 = 下标、墓碑 = `None`），**号是唯一的直接坐标**：名字只到 `seek` 那一格。
//!   核心、载体、服务三层都在（iii 之后服务那一层是**编排域里的一枚线程** + `service.rs` 装配里那一格）。
//!   与 [`system::board`] 并存——那是**另一件事**（板只答"这一位还没了没有"：死信号那一件；
//!   名字→入口归树、生命周期归 [`system::control`]），不是它的旧版。
//!
//!   **"谁能动这一格"分两条轴**：**用**那一轴在门禁那一问里
//!   判（`Rule`：公开 / 就是某一位 / 在某一支里 / 在某枚盟里 / 就是开着某一格的那位），
//!   **改**那一轴记在持树者那本账上（`Ledger` 的 `Owner` + `claimable`）。树自己两轴都不判。
//!
//! [`communication`] 是 `system` 起服务时等就绪的那一步（逐条 `claim` 那本账）；[`system::board`] 是**死信号那一件**
//! ——板线程住 `programs/src/system/board/`（招待所有客人，见该模块"板为什么就一枚线程"），
//! 而**它的客侧也跟着它去了实现侧**（`programs::system::board::client`：`BERTH` / `enroll` /
//! `register` / `evict`）。本层只留它的帧、记号与失败域。
//!
//! **照实记（这一格原先指本 crate 的客侧）**：`system::board::client` 曾住本层。按"耦合了
//! 多个部分、功能不干净就是毒"的判据，board 那条会话上焊着三件事（名字→入口 / 谁还活着 /
//! 待客账），而命名归 [`system::operator`]、生死归 [`system::control`] ⇒ 客侧与语义降回实现侧，
//! 本层不再有那个模块（故这条指路也改指实现侧）。
//!
//! [`system::principal`] 的**地址**走树（`/sys/principal`）：板那一侧已经"照实记"把命名交给树
//! （板只管生死与牌子）。装配者不必查——身份服务起手就把门牌那一枚交给它的生我者，故装配期
//! 每一条服务的 `derive` + `bind` 都在放行之前做完（`service::assemble`）。
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

// 配给那一段与几本账都是**可增长的**（`Vec::try_reserve`，备不下就如实报，不 panic）——与
// `env` / `runtime` 同款：这里引 `alloc`。
extern crate alloc;

// **照实记（`reserve_reads!` 那支宏与它的文件已退场）**：这里原先是 `#[macro_use] mod
// reserve_reads;` —— 一支按格名铺开"`Reserve` 三格一组"那三具身体的宏，理由是"板、树、线、
// 货四家都要用"。今天**只剩 `communication::establish` 一家**（板与树那两处取名层早已撤），
// 宏的 `$vis` 那一格（"共享体与领域名分家"）也无事可做 ⇒ 按"一处形状不值一支宏"拆成三具寻常
// 函数，宏与那一份文件一并撤。

pub mod communication;
pub mod debug;
pub mod driver;
pub mod fail_codes;
pub mod frame;
pub mod id;
pub mod message;
pub mod system;

/// **答话那一格的"没失败"**（0）——全协议**一个号**：六家（principal / coalition / operator /
/// board / line / supply）与驱动各自那几族（如 `programs::driver::rtc`）共用。
///
/// 定义在 [`fail_codes`] 那一份源里（`fail_codes!` 的第二个参数就是它）；这里把它**转出**
/// crate：`fail_codes` 那个模块自己是有意不进公共面的（出 crate 的只有那个宏），而驱动那一侧
/// 的具体协议住在 `programs` 里，要读这一格只能从 crate 根进来。
pub use fail_codes::OK;

// 调试面那一支宏（`debug!`）住 `debug.rs`——**只在 debug 构建下有效**（见那个文件的头注）。
// 它拿 `format!` 拼行，故把 `alloc` 那一支在这里转出：调用方（`programs` / `harness`）因此
// 不必自己先有 `alloc` 这个前提（与上面 `OK` 的转出同一条规矩）。
#[doc(hidden)]
pub use alloc::format as __format;

// ── 面不相撞：三条路的回信孔记号两两不同（**编译期**钉住）──────────────
//
// `principal-back` / `coalition-back` / `line-back`：同一张表里两面的回信孔若刻同一个记号，
// 就分不出这一枚是哪一面的。三对里 `principal ↔ coalition` 那一对钉在
// `system::principal::frame`（那一对），**跨到 `driver::line` 的这两对钉在这里**
// ——`frame.rs` 那两份只认得 `env` 与同层 `core`，看不见 `driver`。这一处看得见整棵树，故由它钉。
//
// **照实记（这两对原先一直是空的）**：三条断言原先都写作 `Mark::of("board-back")`，而**那个名字
// 从来没有存在过**——板那条路的答话走板路那一枚（`system/board/client.rs`：问话孔只写、答话从板路
// 读），它没有 `*-back` 记号。故换成真在的那一条。
const _: () = assert!(
    crate::system::principal::frame::BACK.get() != crate::driver::line::frame::BACK_MARK.get()
);
const _: () = assert!(
    crate::system::coalition::frame::BACK.get() != crate::driver::line::frame::BACK_MARK.get()
);
