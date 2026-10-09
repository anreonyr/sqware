//! 一具寄存架：一枚页上 N 格，满了按策略丢。
//!
//! 与 `ipc::hand` 的关系：那边是**一手**（一枚孔上一条报，交完即走，满了只答
//! `Busy`）；这边把"同样的字节过边界"换成一枚**页**上的有界环（寄存），满了**按 `Mode`
//! 丢一头**并把丢掉的数记在册。两档共用 `wire::Message` 那四样
//! （`Buf` / `EMPTY` / `store` / `fetch`），但**不共用类型**：失败域不同（`Full` 对 `Unbound`）。
//!
//! **一具架只有一枚 Pie**：那枚页上带着"有事"这一位（`Ring`/`Hush`/`Wait` 都认它，见
//! `env::abi::call` 的 `MailCall` 与 `kernel::work::mail::pole`）——**页上那一位就是它的铃**，
//! 故树上一格门牌（一格一枚 Pie）挂得下的正是一具完整的架。
//!
//! # 这一层的四个文件（缓冲与通知正交，两端各自把它们拼起来）
//! ```text
//! mod.rs    契约：Mode · Rack · open/writer/reader/ship/slot（本文）
//! ring.rs   缓冲：页布局、两只游标、四个计数 —— 只借 View，不发系统调用、不分配
//! bell.rs   通知：**页上那一位**的三拍（响 / 等 / 应）—— 只发 Mail 三拍，不碰页里的字节
//! writer.rs 写端 ┐ 把 ring 与 bell 拼成一端；"满了丢哪头"与"取空了应铃"这两条协议住这里
//! reader.rs 读端 ┘
//! ```
//!
//! # 三条不变量
//! 1. **发送端永不挂起**：`send` 只有"放进去了"与"按策略丢了"两种下场 ⇒ 写者不会被慢读者
//!    堵住（对照 `hand::Sender::reclaim` 那条无期等）。
//! 2. **架是通知，不是账**：丢了几个由 `lost`（写端顶掉未读格）＋ `dropped`（`Mode::Newest`
//!    下没进架）两个数说出来；权威状态仍在拥有方那边（树那一侧是 `tile` / `token` / `list`）。
//! 3. **一页一端**：`Rack` 只开**一处**映射，读写两端各从它取一份；同一端不该再交出去
//!    第二份（页是多映射的，这条是用法纪律，类型保证不了——两端各只有一个写者/读者才成立）。
//!
//! **两只游标各只有一个写者**（`write` 归写端、`read` 归读端）是这一族成立的前提，
//! 它的正文在 [`ring`] 的头注里。
//!
//! # 与 `std::sync::mpsc` 的对应
//! `Rack` ↔ `sync_channel(n)`（那对句柄的所有者）；`Writer` ↔ `SyncSender`（但**不阻塞**、
//! 也没有 `Disconnected`——断了的下场是"写不进去"，说法是 `Full` 与 `lost` 计数）；
//! `Reader` ↔ `Receiver`（多一位"有事"：`wait` 等它，醒来还是要读环）。

mod bell;
pub mod reader;
mod ring;
pub mod writer;

use core::marker::PhantomData;

use env::{PieResult, PieToken};
use ::resource::dock::Dock;

use wire::Message;

pub use self::reader::{Reader, RecvFail};
pub use self::ring::{CAP, SIZE};
/// Reader progress wakes a producer retaining a frame under backpressure.
pub const SPACE_BIT: env::Bit = match env::Bit::of(1) { Some(bit) => bit, None => unreachable!() };
pub use self::writer::{SendFail, Writer};

/// 满了丢哪一头。默认 [`Mode::Oldest`]：最新那一格永远保得住（"树变了"比"从前变过"值钱）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// 丢最旧那一格（读者靠格序号跳过去，记 `lost`）。
    Oldest,
    /// 丢正要落的这一格（最新那一格进不来，记 `dropped`）。
    Newest,
}

/// **一具架**：一枚页 —— 而**页上那一位"有事"就是它的铃**。
///
/// 页是架本体（多映射：读写两端映同一段物理帧），那一位是"有事"（写者每落一格响一次、
/// 读者等它）。**铃并进页之后一具架只剩一枚 Pie** ⇒ 树上一格门牌挂得下的正是它（见
/// `env::abi::call` 的 `Ring`/`Hush`/`Wait` 与 `pole::PoleMeta` 那一节）。
/// 两端由这一具架取（`writer()` / `reader()`），或由 [`Rack::ship`] 交出的**那一枚号**在对端
/// 重建（[`Writer::from_raw`] / [`Reader::from_raw`]，与 `Sender::from_raw` 同形）。
pub struct Rack<M: Message> {
    /// 页（借映进本域，与 `Dock` 同一条手：`open` 返视图、`shut` 撤图）。
    dock: Dock,
    /// 满了丢哪一头（写那一侧的规矩，读端不必知道）。
    mode: Mode,
    _m: PhantomData<M>,
}

impl<M: Message> Rack<M> {
    /// 开一具架：**一枚清过零的共享页**（页上那一位从"没响"起），`mode` 定满了丢哪一头。
    ///
    /// # Errors
    /// 页解封不出来 / 映不进来（`PieFail`）——起手那一步没材料，调用方按"这一档用不了"处置。
    pub fn open(mode: Mode) -> PieResult<Self> {
        ring::exact::<M>();
        // **起手那一步报一行（release 也看得见）**：`debug!` 在 release 是空操作，
        // 而"页解不出来"是这一档用不了的直接成因。
        let pie = match env::pie::unseal(env::UnsealArgs::Pole { size: SIZE, shared: true }) {
            Ok(pie) => pie,
            Err(fail) => {
                crate::debug::put(&alloc::format!("rack: no page {:?}", fail));
                return Err(fail);
            }
        };
        let dock = match Dock::open(pie) {
            Ok(dock) => dock,
            Err(fail) => {
                crate::debug::put(&alloc::format!("rack: no view {:?}", fail));
                return Err(fail);
            }
        };
        // 页是内核清零的（`UnsealPole` 的正文），故四个计数与每格 `seq` 都从 0 起。
        Ok(Self {
            dock,
            mode,
            _m: PhantomData,
        })
    }

    /// 写端（与 std 的 `SyncSender` 同位）。
    pub fn writer(&self) -> Writer<M> {
        Writer::of(self.dock.view(), self.mode, self.dock.pie_token())
    }

    /// 读端（与 std 的 `Receiver` 同位）。
    pub fn reader(&self) -> Reader<M> {
        Reader::of(self.dock.view(), self.dock.pie_token())
    }

    /// **交出去的那一枚号**：就是这枚**页**（页上那一位即铃）。对端拿它重建自己的那一端。
    pub fn ship(&self) -> PieToken {
        self.dock.pie_token()
    }

    /// **环里第 `seq` 格此刻的载荷字节**（号从 1 起，[`Writer::send`] 每落一格加一）。
    ///
    /// 写者刚写完就把它**借出去**（交给孔上那只手：手只登记"这一段在哪"，取走那一刻内核复制
    /// 一次）。**只在环没绕回这一格之前有效**：绕回来之后这里就是更新的内容——故载荷里带一格
    /// 自己的号（`Event::seq`），读者自己辨"这一条我读过没有、中间丢了几条"。
    pub fn slot(&self, seq: u64) -> &[u8] {
        ring::payload_at(ring::ring(self.dock.view()), seq)
    }
}
