#![no_std]
//! protocol — 用户态协议层。
//!
//! 重新开张的条件只有一条：**一个任务要让另一个任务做事**。
//! 在那之前不需要协议——跨域要说的话走 `env` 的调试面（`DebugCall`：内核把固件的调试
//! 控制台直接借给域），一条孔都不用开。`echo` 就是这么说话的。
//!
//! # 这一层装什么：**两句**
//!
//! 1. **给别的 task 用的一切** —— 正文、判定、帧、**客侧那几手**（`*/client.rs`）：从外面找上
//!    某份协议的人**只需要这一面**；
//! 2. **能在宿主上跑的实现** —— 只判、只记、不发消息的那几份（`operator` 的 `judge` / `gate` /
//!    `ledger` 是今天的全部）：它们**不是**给别的 task 用的，是**实现方**的东西；住在这里的
//!    理由是**进得了宿主靶**（`crates/protocol-case` 把核心**逐字** `#[path]` 编进来跑判据——
//!    而 `programs` 编不进宿主，拖着 riscv 内联汇编）。
//!
//! **照实记（第二句是补上的那一半）**：原先只写了第 1 句。第 2 句管的是**依赖面**那条判据
//! ——"别的 task 不该依赖实现"——而它今天**事实上成立**（那三份的读者只有持树者与宿主靶两处，
//! `grep` 得出来），差的是**文件同住**（它们与客侧那几手在同一个模块树里）。要把文件也分开，
//! 得先找一处**既能被宿主靶逐字编、又算实现**的落脚（新 crate + `programs` 加依赖 + 靶改
//! `#[path]`）——**那是独立的一件事，今天不做**。
//!
//! 与 `programs/src/lib.rs` 的"**判据是谁在说话**"同一条线：这一层是那条线的**接口那一边**，
//! `programs` 是**实现那一边**（实现方跟着"用它那个程序所在的档"走）。
//!
//! **顶层只有三份正文**——这里就是"目录层次"那句话：
//!
//! ```text
//!   session          地板    会话怎么建起来（其余每一份都建在它上面）
//!   system           系统    编排（core / desk / grant）
//!                     └ 容纳  board      运行期的公示板 ＋ 待客账
//!                             operator   命名寻址：一棵树，名字 → Pie
//!                             principal  策略身份：这个 Task 此刻代表谁、从谁而来
//!                             coalition  策略结盟：身份的横向那半（principal 的客人）
//!   driver           轴      物料到手（supply）／线（line）
//!   （根上三件共享件：`frame` 帧骨架 · `id` 号的规则 · `fail_codes` 负码表）
//! ```
//!
//! **照实记（"容纳"是用户裁的）**：`board` 一直在 [`system`] 之下；`operator` / `principal` /
//! `coalition` 原先是**顶层**（与 `system` 平级），裁定之后收进去。**判据是"谁住编排域"**：
//! iii 之后这四套协议的落地（板线程 / 持树者 / 名册 / 盟册）都是**编排域里的线程**，
//! 而 [`system`] 正文自己写着"**要找服务得先有目录**——今天那本目录就是 `board`"。
//! 故**协议树与实现树（`programs/src/system/`）同形**。
//! **被否的那条读法**是"协议树按'谁在说话'分、不该镜像实现树"（我原先的建议）——用户裁的是前者。
//!
//! 落地程度不一样：**三份顶层 ＋ 容纳的四套都已经有代码跑在机器上**（[`system`]、[`driver`]
//! （**两半都落了**：`supply` 与 `line`——见 [`driver::line`]，四格原语 + 账 + 客侧几手，
//! `router` / `uart` / `rtc` 与两位客人 `lodger` / `sleeper` 都跑在机器上）、[`session`]、
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
//!   （`prog-system`）：`system/desk.rs` 是那张服务表，`programs/.../system/`
//!   是它落地的那一台。起它的那一枚（引导域 `root`）只做**固件那一层**的事：读 boot 的
//!   两块账、把字节与门闩按单子交出去、退出即停机——它不认识服务名，也不记账。
//! - [`driver`] = **设备轴**：一台设备从"交到某个域手里"到"它的线有人领"这一整段。
//!   **两半**：**物料到手**（`supply`——引导域向上层露的那一面，"一张单子换一段记录"，
//!   按坐标发货、原件与 `VEST` 都留在引导域手里；两个角色：`server`（引导域的发货循环）、
//!   `client`（编排域去领））与**线**（权威 / 属主 / 登记 / 投递 / 排空 / 收线——**四格都落在
//!   机器上**，见 [`driver::line`]）。
//!   它不是驱动框架：设备语义各驱动自带（`programs/src/driver/<域>/`），装配样板住程序侧
//!   （`programs/src/driver/assemble.rs`），本层只管**跨域约定**。
//! - [`system::principal`] = **策略身份**：**名册**（TID → 此刻代表的 PrincipalId）与**谱系**
//!   （PrincipalId 的一棵只增不改的树）。两条轴都不定义权限——收到它的服务自己解释那条号。
//!   载体建在会话之上：门牌落在树上 `/sys/principal`，一问一答替这一趟借一枚回信孔过去。
//! - [`system::coalition`] = **策略结盟**（策略身份那一条轴的**横向**那半）：**一张两列表**——哪条身份
//!   在哪些盟里、哪枚盟里有谁，反着念是同一个关系的两个方向。号由服务铸（铸过就一直在），
//!   盟无主（故失败域里没有 `Denied`），不产生 PrincipalId、不发 Pie、不解释成员资格的含义。
//!   六条原语（`found` / `enter` / `leave` / `amid` / `band` / `bloc`）——**六条都在线上**
//!   （`call.rs` 那一排码 `1..6`，BAND / BLOC 那两条已由服务实现）。
//!   照实记：这一句原来写的是"四条上线、两条住核心"，那是 `band` / `bloc` 还没接上时的口径。
//!   它是**身份服务的客人**：每条写原语嵌一次 `Resolve(发送者)`——"self"因此在适配层，
//!   不在核心（正文的"已知边界"里写着这一条的确切含义）。
//! - [`session`] = **会话建立**："两个陌生实体怎么建起一条会话"。身份由内核盖、地址靠
//!   对方交、认领按"谁开的这扇门"——不需要 Server 就能成立，而其余几份都建在它上面。
//! - [`system::operator`] = **命名寻址（树那一版）**：一个 Operator 管着所有条目，其他任务只是
//!   操作它——**对外七条**：`land` 落 / `part` 分 / `find` 寻 / `trim` 剪 / `list` 列 /
//!   `seek` 译 / `name` 名（外加核心那一条只读 `opens`，不上线）。树是**一张按号排的表**
//!   （号 = 下标、墓碑 = `None`），**号是唯一的直接坐标**：名字只到 `seek` 那一格。
//!   核心、载体、服务三层都在（iii 之后服务那一层是**编排域里的一枚线程** + `service.rs` 装配里那一格）。
//!   与 [`system::board`] 并存——那是**另一件事**（公示板 + 待客台账），不是它的旧版。
//!
//!   **"谁能动这一格"分两条轴**：**用**那一轴在门禁那一问里
//!   判（`Rule`：公开 / 就是某一位 / 在某一支里 / 在某枚盟里 / 就是开着某一格的那位），
//!   **改**那一轴记在持树者那本账上（`Ledger` 的 `Owner` + `claimable`）。树自己两轴都不判。
//!
//! [`session`] 是 `system` 起服务时等就绪的那一步；[`system::board`] 是 `programs` 里那**一枚**
//! 板线程（招待所有客人，见该模块"板为什么就一枚线程"）+ 编排域里共享的那一份板
//! （服务怎么问在 [`system::board::client`]；板那一台与装配侧在
//! `programs/src/system/board/`）。
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

// 码头的泊位是**一张可增长的账**（`session::core::Quay`）：条数由调用方按路数决定，
// 故本 crate 引 `alloc`（与 `env`/`runtime` 同款；备不下时由 `Vec::try_reserve` 如实报
// `Seat::NoRoom`，不 panic）。
extern crate alloc;

// ── 术语与它的两支宏 ───────────────────────────────────────
//
// 两支宏只做一件事：**把一份身体按格／按表铺开**——一处裁决都不放。术语全部从地板取
// （`env::fid::PieCall::Reserve` 的三格名 `vestor` / `owner` / `mark`、线上答话那一格），
// 不自造。住 crate 根是因为板、树、线、货四家都要用，而本仓**不为一个形状新开文件**。

/// `Reserve` 的三格：**一个调用的三个事实**，按格展开——一格一个读出。
///
/// 三格的读法只此一份（`mail::reserve` 那一问 + 两个哨兵）：`owner == 0`（引导期那批设备
/// 门闩）不算"谁开的"；记号答不出就报 `None`。调用点只给**格名**（`vestor` / `owner` /
/// `mark`）与**自己那一侧的名字**，故"三格是一组、三个名字等长"在调用处一眼可见——
/// 原先板与树各写一份 `probe`5 / `opened_by`9 / `mark_of`7，不等长本身就是信号。
///
/// `$vis` 那一格是给**共享体与领域名分家**用的：身体住 `session::call`，名字由调用点给。
macro_rules! reserve_reads {
    ($(#[$meta:meta])* $vis:vis fn $name:ident($arg:ident) => vestor $(;)?) => {
        $(#[$meta])*
        $vis fn $name($arg: env::PieToken) -> Option<env::TaskId> {
            runtime::env::mail::reserve($arg)
                .ok()
                .map(|(vestor, _owner, _mark)| vestor)
        }
    };
    ($(#[$meta:meta])* $vis:vis fn $name:ident($arg:ident) => owner $(;)?) => {
        $(#[$meta])*
        $vis fn $name($arg: env::PieToken) -> Option<env::TaskId> {
            match runtime::env::mail::reserve($arg) {
                Ok((_vestor, owner, _mark)) if owner.get() != 0 => Some(owner),
                _ => None,
            }
        }
    };
    ($(#[$meta:meta])* $vis:vis fn $name:ident($arg:ident) => mark $(;)?) => {
        $(#[$meta])*
        $vis fn $name($arg: env::PieToken) -> Option<env::Mark> {
            match runtime::env::mail::reserve($arg) {
                Ok((_vestor, _owner, mark)) => Some(mark),
                _ => None,
            }
        }
    };
}

// 码表宏自己一份源：协议与**宿主靶**同读这一份（宿主靶不依赖 `protocol`，见那份文件的照实记）。
//
// `#[macro_use]` 与 `#[macro_export]` 两样都要：前者把宏带进**本 crate 后面那些模块**的作用域
// （宏的可见性按正文先后），后者保住"出 crate"那一份（`protocol::fail_codes!`）——**调用点
// 因此一行都不用改**（照实记：先只留 `#[macro_export]`，编出来 11 处 "cannot find macro"）。
#[macro_use]
mod fail_codes;

/// **答话那一格的"没失败"**（0）——全协议**一个号**：六家（principal / coalition / operator /
/// board / line / supply）与驱动各自那几族（如 `programs::driver::rtc`）共用。
///
/// 定义在 [`fail_codes`] 那一份源里（`fail_codes!` 的第二个参数就是它）；这里把它**转出
/// crate**：`fail_codes` 自己是有意私有的（出 crate 的只有那个宏），而驱动那一侧的具体协议住在
/// `programs` 里，要读这一格只能从 crate 根进来。
///
/// **各家的失败码不共用**（同一个概念在两家是别的号，见各族 `frame.rs` 的注）——共用的只有
/// "没失败"那一格。
pub use fail_codes::OK;

pub mod driver;
pub mod frame;
pub mod id;
pub mod session;
pub mod system;

// 依赖先留着：`env` 与 `runtime` 是地板，第一条协议操作出现时立刻要用。
// （本文件自己只有：下面那条 `reserve_reads!` 宏、`mod fail_codes`、五个 `pub mod` 与那两条
// 编译期断言——`env` / `runtime` 只出现在**宏体**里，由调用宏的那些模块去用 ⇒ `cargo` 若在
// **本文件这一格**报 unused dependency，是预期噪音。）

// ── 面不相撞：三条路的回信孔记号两两不同（**编译期**钉住）──────────────
//
// `principal-back` / `coalition-back` / `line-back`：同一张表里两面的回信孔若刻同一个记号，
// 就分不出这一枚是哪一面的。三对里 `principal ↔ coalition` 那一对钉在
// `system::principal::frame`（那一份的宿主靶两边都在），**跨到 `driver::line` 的这两对钉在这里**
// ——`frame.rs` 那两份要能在宿主靶里**逐字单独编**，而那两个靶的模块树里没有 `driver`（它们
// 只认得 `env` 与同层 `core`）。这一处看得见整棵树，故由它钉。
//
// **照实记（这两对原先一直是空的）**：三条断言原先都写作 `Mark::of("board-back")`，而**那个名字
// 从来没有存在过**——板那条路的答话走码头（`system/board/client.rs`：问话孔只写、答话从板路
// 读），它没有 `*-back` 记号。故换成真在的那一条。
const _: () = assert!(
    system::principal::frame::BACK.get() != driver::line::call::BACK_MARK.get()
);
const _: () = assert!(
    system::coalition::frame::BACK.get() != driver::line::call::BACK_MARK.get()
);
