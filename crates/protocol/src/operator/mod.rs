//! Operator Protocol — **命名寻址**：一棵树，名字 → Pie。
//!
//! 全系统**就一个** Operator，管着所有条目；其他任务只是**操作**它：
//!
//! ```text
//!   file  挂   把一枚 Pie 挂到一个名字上（放一个文件）
//!   tile  铺   开一块空文件夹
//!   find  寻   走到头，把那一枚 Pie 交出去
//!   trim  剪   剪掉一条
//!   list  列   看一块文件夹里有哪些名字
//! ```
//!
//! # 树
//!
//! ```text
//!   Entry { 名字, 去处 }          名字 = 一段
//!   Node  = Tile(Vec<Entry>)      文件夹：还能往里走
//!         | File(PieToken)        文件：到头了，就是内核那一枚
//!   根    = 顶层那个 Vec<Entry>    空路 = 根
//! ```
//!
//! 一条路是**段列表**（`&[Name]`），不是一个字符串：一段就是现成的 `Name`（定长 32 字节、
//! 构造即校验）——于是"名字不合法"在类型上不存在，也没有分隔符 / 转义 / `..` 这些边界。
//!
//! # 五条原语
//!
//! | 原语 | 干什么 | 落在一枚条目上 |
//! |---|---|---|
//! | [`Operator::file`] | 挂 | 放一条 `File`：最后一段空着就挂上，占着就是换绑 |
//! | [`Operator::tile`] | 铺 | 放一块空 `Tile`（新建文件夹） |
//! | [`Operator::find`] | 寻 | 走到头，把那一枚 Pie 交出去 |
//! | [`Operator::trim`] | 剪 | 把路上那一条剪掉 |
//! | [`Operator::list`] | 列 | 读一块 `Tile` 里的名字 |
//!
//! **要动一条 `Tile`，先把它清空**——`file`（换绑）与 `trim` 是同一条规矩：
//! 非空 `Tile` ⇒ [`Fail::NonEmpty`]。
//!
//! # 失败域只有六格
//!
//! [`Fail`]：`Unknown` / `NonEmpty` / `NotAFile` / `NotATile` / `Full` / `Dead`——每格对应一个
//! **不同的下一步**。
//!
//! **没有"名字已被占"那一格**：同名接手一条 `File`、或一块**空的** `Tile`，都是换绑；而 owner
//! 归 Principal，Operator 分不出"自己 / 别人"，"已占即拒"在这里无处落脚。
//!
//! # 签名里没有"谁"
//!
//! 五条原语都没有 caller 参数、条目也没有 owner 字段：**谁能动由 Principal 那一层回答**
//! （内核在 Push 时盖的发送者印章是现成的，Principal 拿去用，Operator 不看它）。
//!
//! # 两个注入的事实
//!
//! [`Probe`]（那一枚 Pie 还答得出吗）与 [`Free`]（把我这一份放下——剪掉或换掉一条 `File` 时用它，
//! 不加这一格那一枚句柄就漏在树里）。核心因此不 `use` 内核，喂两个假闭包就能把规矩推理干净。
//!
//! # 与 [`board`](crate::board) 的关系
//!
//! `board` 是**另一件事**：一块公示板 + 一本待客台账（名字可以立着而没有东西、记得"谁挂的"、
//! 有死亡道、摘牌子不流转）。这一份是**一棵命名树**：结构长在它自己里面、不记 owner、
//! 谁问都答。两处并存，去留不在本正文里定。
//!
//! # 落地程度
//!
//! 三层都在：**核心**（[`core`]：树 + 五条原语 + 宿主用例）、**载体**（[`call`] 的帧与转发、
//! [`desk`] 的客人小账）、**服务**（`programs/src/bin/supervisor/operator.rs` 的
//! `serve` / `attach` / 客侧三手，加 `prog-operator` 这个域；装配那一格在
//! `programs/.../service.rs` 的 `Program::operator`）。
//!
//! **`list` 没有上线**：它的答案是一串名字，要另开一种帧形（今天只有一问一答一格状态那种），
//! 故线上只有 `file` / `tile` / `find` / `trim` 四码——`Operator::list` 仍住核心，只给本域
//! 自己与宿主用例用。

pub mod call;
pub mod core;
pub mod desk;

pub use core::{Entry, Fail, Free, Node, Operator, Probe};
pub use desk::{Desk, Guest};
