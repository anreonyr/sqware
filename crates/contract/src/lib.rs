#![no_std]
//! contract — **约**：两个 task 之间**那一句话本身**（形 · 据 · 账 · 不碰内核的适配）。
//!
//! # 划界只有一条判据
//!
//! > **本 crate 全树不碰 `runtime` 那一层。**
//!
//! 这条纪律从前写在**六份文件的头注**里（`system/board/core.rs`、`system/operator/core.rs`、
//! `session/core.rs`、`driver/line/core.rs`、`programs/.../board/desk.rs`、
//! `programs/.../operator/desk.rs`）——**有纪律，没有边界**。现在它是这个 crate 的存在理由：
//! 不碰内核的一切住这里，**宿主靶直接依赖本 crate**，不必再靠 `#[path]` 把源码逐字搬进测试。
//!
//! 碰内核的那一面住 `protocol`（**口**）：客侧那几手、会话那几手、落内核的适配。
//! 一句话记：**协议是话，程序是说话的人**——域入口、那一台、装配单、需求单、死法、装配次序
//! 都住 `programs`。
//!
//! # 面上有什么（分批搬进来；这里是**第一批**）
//!
//! ```text
//!   frame       形：principal 与 coalition **同形的那一份**骨架
//!   id          号：`Id` 那一族怎么编、怎么读
//!   fail_codes  负码表：`fail_codes!` 宏 ＋ 全协议共用的那一格 `OK`
//! ```
//!
//! **照实记（这三件为什么同批）**：`frame` 要 `id` 与 `fail_codes` ⇒ 三件是一组，只搬一件编不过。
//! 搬完 `protocol` 用 `pub use contract::{frame, id};` 与 `pub use contract::fail_codes::OK;`
//! **转出** ⇒ 调用点一处不改；宿主靶**真依赖**本 crate 取它们。
//!
//! # 面会长成什么样（后面几批）
//!
//! ```text
//!   正文   `mod.rs`（薄：这句话是什么 ＋ 不变量）
//!   形     `frame.rs`
//!   据     `core.rs`
//!   账     `desk.rs`
//!   适配   `call.rs`（**不碰内核**的那几手：立板、注入、对照表）
//! ```
//!
//! **不进来**：`client.rs` 的客侧那几手、`session/call.rs` 的会话手——它们碰内核，住 `protocol`。
//! **照实记（一处按手切、不按文件切）**：`system/{board,operator}/call.rs` 现在不碰 `runtime`，
//! 但它 `pub use` 的三手（`marked_as` / `opened_by` / `vested_by`）由 `reserve_reads!` 包着
//! `mail::reserve`——**是内核读**。搬它们那一批时按手劈：立板与两张对照表进本 crate，
//! `ship` 与那三手进口。
//! # 这一层装什么：**两句**
//!
//! 1. **给别的 task 用的一切** —— 正文、判定、帧、**客侧那几手**（`*/client.rs`）：从外面找上
//!    某份协议的人**只需要这一面**；
//! 2. **能在宿主上跑的实现** —— 只判、只记、不发消息的那几份（`operator` 的 `judge` / `gate` /
//!    `ledger` 是今天的全部）：它们**不是**给别的 task 用的，是**实现方**的东西；住在这里的
//!    理由是**进得了宿主靶**（`crates/protocol-case` **真依赖**本 crate 来跑判据——而
//!    `programs` 编不进宿主，拖着 riscv 内联汇编）。
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
//! **`frame.rs` 只在"帧那一半要能被单独编"时才单开**（真凭据：[`system::principal::frame`]
//! 头注写着那个宿主靶"模块树里没有 `driver`"，[`system::operator::gate`] 同理）。`driver` 那
//! 两半（[`driver::supply`] / [`driver::line`]）的帧**整份都在宿主靶的判据里**（`crates/protocol-case/
//! tests/{supply,line}.rs` 跑的就是那一份）⇒ 没有分家的需要。**故同一个文件名在两族里指两件
//! 事**：`system/*/call.rs` 是**转发那几手**，`driver/*/call.rs` 是**形状与记号**。名字不并
//! （改名要动四十余处引用，换一条对称），差异由这一句兜住。
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


// 配给那一段与几本账都是**可增长的**（`Vec::try_reserve`，备不下就如实报，不 panic）——与
// `protocol` 同款：这里引 `alloc`。
extern crate alloc;

pub mod driver;
pub mod fail_codes;
pub mod frame;
pub mod id;
pub mod session;
pub mod system;

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
    crate::system::principal::frame::BACK.get() != crate::driver::line::frame::BACK_MARK.get()
);
const _: () = assert!(
    crate::system::coalition::frame::BACK.get() != crate::driver::line::frame::BACK_MARK.get()
);
