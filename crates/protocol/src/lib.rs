#![no_std]
//! protocol — 用户态协议层。
//!
//! 重新开张的条件只有一条：**一个任务要让另一个任务做事**。
//! 在那之前不需要协议——跨域要说的话走 `env` 的调试面（`DebugCall`：内核把固件的调试
//! 控制台直接借给域），一条孔都不用开。`echo` 就是这么说话的。
//!
//! **目前有六份顶层正文**：[`system`]、[`driver`]、[`principal`]、[`coalition`]、[`session`]
//! 与 [`operator`]。
//!
//! 落地程度不一样：**六份都已经有代码跑在机器上**（[`system`]（含它下面的 `board`）、
//! [`driver`]（**两半都落了**：`supply` 与 `line`——见 [`driver::line`]，四格原语 + 账 +
//! 客侧几手，`router` / `uart` / `rtc` 与两位客人 `lodger` / `sleeper` 都跑在机器上）、
//! [`session`]、[`operator`] 与 [`principal`]（**名册 + 谱系**：七条原语、一台
//! `prog-principal`、一位真客人 `subject`））。
//!
//! - [`system`] = **服务编排**（systemd 那一层）：系统由哪些 Service 构成、怎么起停监督。
//!   它**有两半**：**编排**（`core` / `desk` / `grant`）与**运行期命名**（[`system::board`]：
//!   "这个名字此刻指向哪个入口"——一块公示板、一枚牌子、三个动作，判据只有一条：那枚入口
//!   是你亲手交给持板者的，**不存预约表**）。两半同住一份正文，因为板线程**就住在编排域里**
//!   （`root` 那张单上没有板那一格）：名字对上入口的静态那半是 `grant`，运行期这半是 `board`。
//!   它的载体是内核 ABI（`env::fid` 的 `UnitCall` 整类 + `RoomCall` 的 `Reap`/`Doom`），
//!   那份载体叙述整体降级为该模块的**附录**——载体不等于协议。**它的 Server 是一个独立域**
//!   （`prog-system`）：`system/desk.rs` 是那张服务表，`programs/.../supervisor/system/`
//!   是它落地的那一台。起它的那一枚（引导域 `root`）只做**固件那一层**的事：读 boot 的
//!   两块账、把字节与门闩按单子交出去、退出即停机——它不认识服务名，也不记账。
//! - [`driver`] = **设备轴**：一台设备从"交到某个域手里"到"它的线有人领"这一整段。
//!   **两半**：**物料到手**（`supply`——引导域向上层露的那一面，"一张单子换一段记录"，
//!   按坐标发货、原件与 `VEST` 都留在引导域手里；两个角色：`server`（引导域的发货循环）、
//!   `client`（编排域去领））与**线**（权威 / 属主 / 登记 / 投递 / 排空 / 收线——**四格都落在
//!   机器上**，见 [`driver::line`]）。
//!   它不是驱动框架：设备语义各驱动自带（`programs/src/driver/<域>/`），装配样板住程序侧
//!   （`programs/src/driver/assemble.rs`），本层只管**跨域约定**。
//! - [`principal`] = **策略身份**：**名册**（TID → 此刻代表的 PolicyId）与**谱系**
//!   （PolicyId 的一棵只增不改的树）。两条轴都不定义权限——收到它的服务自己解释那条号。
//!   载体建在会话之上：门牌落在树上 `/sys/principal`，一问一答替这一趟借一枚回信孔过去。
//! - [`coalition`] = **策略结盟**（策略身份那一条轴的**横向**那半）：**一张两列表**——哪条身份
//!   在哪些盟里、哪枚盟里有谁，反着念是同一个关系的两个方向。号由服务铸（铸过就一直在），
//!   盟无主（故失败域里没有 `Denied`），不产生 PolicyId、不发 Pie、不解释成员资格的含义。
//!   六条原语（`found` / `enter` / `leave` / `amid` / `band` / `bloc`），四条上线、两条住核心。
//!   它是**身份服务的客人**：每条写原语嵌一次 `Resolve(发送者)`——"self"因此在适配层，
//!   不在核心（正文的"已知边界"里写着这一条的确切含义）。
//! - [`session`] = **会话建立**："两个陌生实体怎么建起一条会话"。身份由内核盖、地址靠
//!   对方交、认领按"谁开的这扇门"——不需要 Server 就能成立，而其余几份都建在它上面。
//! - [`operator`] = **命名寻址（树那一版）**：一个 Operator 管着所有条目，其他任务只是
//!   操作它——`land` 落 / `part` 分 / `find` 寻 / `trim` 剪（`list` 未上线），落在那棵
//!   `Entry { 号, 名字, 去处 }`、`Node = Pane | Tile` 的树上。核心、载体、服务三层都在
//!   （`prog-operator` 一个域 + `service.rs` 装配里那一格）。与 [`system::board`] 并存——
//!   那是**另一件事**（公示板 + 待客台账），不是它的旧版。
//!
//! [`session`] 是 `system` 起服务时等就绪的那一步；[`system::board`] 是 `programs` 里那**一枚**
//! 板线程（招待所有客人，见该模块"板为什么就一枚线程"）+ 编排域里共享的那一份板
//! （服务怎么问在 [`system::board::client`]；板那一台与装配侧在
//! `programs/src/supervisor/system/board/`）。
//!
//! [`principal`] 的**地址**走树（`/sys/principal`）：板那一侧已经"照实记"把命名交给树
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

/// 码表：**失败域 ↔ 线上答话那一格**，四家同一个形状。
///
/// - 正向一律 `fail_to_code(Option<Fail>) -> u8`：`None`（没失败）⇒ `OK`；
/// - 反向一律 `code_to_fail(u8) -> Option<Fail>`：`OK` ⇒ `None`。
///
/// **反向只在双射时生成**。非双射的表（几种失败归同一个码）**不给反向**——由人写并注明
/// "反不回来"。这是判据，不是风格：给非双射的表生成反向，等于把"对偶"说成假的。
///
/// **出 crate**（`#[macro_export]`）：第二个实例到了——驱动的**具体协议**住各驱动自己的目录
/// （那一条裁定见 [`driver`]），而它同样要一张"失败域 ↔ 线上那一格"的表。手抄一遍就是两处编。
#[macro_export]
macro_rules! fail_codes {
    ($(#[$meta:meta])* bijective $fail:ty; $ok:ident; $($variant:path => $code:ident),+ $(,)?) => {
        $(#[$meta])*
        pub const fn fail_to_code(fail: Option<$fail>) -> u8 {
            match fail {
                None => $ok,
                $(Some($variant) => $code),+
            }
        }

        /// 线上答话那一格 → 失败域。`OK`（没失败）那一格一定答 `None`——读的人靠动作码先分流。
        ///
        /// **`BAD`（这一问读不懂）在不在表里，由各家自己的表说**：板那一侧它独立一格、留在
        /// 表外（`system::board::call`），`driver::rtc` 那一侧它与"没走到"（`Fail::Denied`）
        /// 合流——那边的持有者从来不说"我没接住"这句话（接不住就是没有孔可回）。
        ///
        /// 表外那一格与读不懂的码一律答 `None`：两个 `None` 不是同一件事，读的人靠动作码先分流。
        pub const fn code_to_fail(code: u8) -> Option<$fail> {
            match code {
                $ok => None,
                $($code => Some($variant)),+,
                _ => None,
            }
        }
    };
    ($(#[$meta:meta])* lossy $fail:ty; $ok:ident; $($arm:pat => $code:ident),+ $(,)?) => {
        $(#[$meta])*
        pub const fn fail_to_code(fail: Option<$fail>) -> u8 {
            match fail {
                None => $ok,
                $(Some($arm) => $code),+
            }
        }
    };
}

pub mod coalition;
pub mod driver;
pub mod operator;
pub mod principal;
pub mod session;
pub mod system;

// 依赖先留着：`env` 与 `runtime` 是地板，第一条协议操作出现时立刻要用。
// （本文件暂时没有代码，故 `cargo` 若报 unused dependency，那是预期内的噪音。）
