//! 读端：读一格就**前进一步**；架空了就**应铃**、再看一眼、然后等。
//!
//! 与 `hand::Receiver` 的分界：那边一次 `recv` 拿到"别人递来的那一条"；这边是**一条队列的
//! 游标**——`try_recv` 不前进（看一眼），`recv` / `try_recv` 读到才前进。格序号比"我该读的
//! 那一号"大 ⇒ 中间那几格被写端覆盖丢了，跳过并把枚数记在**本端**（[`Reader::skipped`]），
//! 页里那枚 `lost` 由写端记（见 [`super::ring`]）。
//!
//! # 游标从页里起
//! [`Reader::of`] / [`Reader::from_raw`] 的起始号取自页里的 `read`（**不是从 1 起**）：
//! 同一具架换一个读端（重启、重建）时不重放已经取走的消息。
//!
//! # 唤醒那一套（为什么不是"读不到就睡"）
//! 铃是一位、`wait` **不清**它 ⇒ "读空了"与"应铃"必须同一刻做，否则醒来时那一位还亮着、
//! 下一趟当场就返（一圈空转）。而"应铃之后再探一次"这一步也不能省：
//!
//! ```text
//! 读（空） → 应铃 → 再读 → 等铃
//!            ↑ 若写端恰在这两拍之间落格响铃，那一下不会被这一次应铃吃掉
//! ```
//!
//! 这三拍都在 [`Reader::recv`] 里，调用方不必自己拼。

use core::marker::PhantomData;

use ::resource::dock::{Dock, View};
use env::{MailFail, PieToken, Wait};

use super::bell::Bell;
use super::ring::{Ring, SLOT, cursor, exact, lost, pop, ring};
use wire::Message;

/// **读端**：一条游标 ＋ 一枚铃。
pub struct Reader<M: Message> {
    ring: &'static Ring,
    /// 本域里那一份映射（`Rack::reader` 现取时是 `None`：`Rack` 持着 `Dock`）。
    /// **它只为"持着"而存在**（见 [`super::writer::Writer`] 同一格的注）。
    _dock: Option<Dock>,
    /// **我该读的那一号**（绝对序号，从页里的 `read` 起）。
    next: u64,
    /// 本端跳过的枚数（页里的 `lost` 由写端记，这一枚是"我看见的"）。
    skipped: u64,
    bell: Bell,
    progress: Bell,
    _m: PhantomData<M>,
}

impl<M: Message> Reader<M> {
    /// 本域里现取一枚读端（`Rack::reader`）：映射归 `Rack`。
    /// `page` = **那一枚页**（页上那一位就是铃：读干了应一下）。
    pub fn of(view: View, page: PieToken) -> Self {
        exact::<M>();
        Self {
            ring: ring(view),
            _dock: None,
            next: cursor(ring(view)),
            skipped: 0,
            bell: Bell::from_raw(page, env::Bit::FIRST),
            progress: Bell::from_raw(page, super::SPACE_BIT),
            _m: PhantomData,
        }
    }

    /// **对端**那边的读端：拿 [`super::Rack::ship`] 交出的**那一枚号**重建（与 `Receiver::from_raw`
    /// 同形）。页映不进来 ⇒ `None`（此后 `try_recv` 一律答 `RecvFail::Mail(Denied)`，不猜地址）。
    pub fn from_raw(page: PieToken) -> Option<Self> {
        exact::<M>();
        let dock = Dock::open(page).ok()?;
        let view = dock.view();
        Some(Self {
            ring: ring(view),
            _dock: Some(dock),
            next: cursor(ring(view)),
            skipped: 0,
            bell: Bell::from_raw(page, env::Bit::FIRST),
            progress: Bell::from_raw(page, super::SPACE_BIT),
            _m: PhantomData,
        })
    }

    /// 看一眼：**不前进**（`Ok(None)` = 这一刻没读到）。
    pub fn try_recv(&mut self) -> Result<Option<M::In>, RecvFail> {
        let mut frame = [0u8; SLOT];
        match pop(self.ring, &mut self.next, &mut self.skipped, &mut frame) {
            Ok(Some(n)) => {
                let _ = self.progress.ring();
                let bytes = frame.get(..n).unwrap_or(&[]);
                match M::fetch(bytes) {
                    Some(in_) => Ok(Some(in_)),
                    None => Err(RecvFail::Unread(n)),
                }
            }
            Ok(None) => Ok(None),
            // 这一格被别的读者拿走了（一页一端，不该发生）：如实报，不猜。
            Err(0) => Err(RecvFail::Mail(MailFail::Busy)),
            Err(n) => Err(RecvFail::Unread(n)),
        }
    }

    /// **等铃**（不 `hush`）：`true` = 有事（醒来还是要回环里读），`false` = 期限内没等到。
    ///
    /// **自己会读干的那一圈别用它**——用 [`Reader::recv`]（它把那三拍拼好了）。
    pub fn wait(&self, within: Wait) -> Result<bool, RecvFail> {
        self.bell.wait(within).map_err(|e| RecvFail::Mail(e.source))
    }

    /// **应铃**：清掉"有待取之事"。已经清着 ⇒ `Err(RecvFail::Mail(Busy))`（调用方当"正好"）。
    pub fn hush(&self) -> Result<(), RecvFail> {
        self.bell.hush().map_err(|e| RecvFail::Mail(e.source))
    }

    /// **等 ＋ 读一条**（与 std 的 `recv` 同位）。
    ///
    /// `within` = 这一次调用等多久（`Wait::POLL` = 只探测一次，**不挂起**）。
    /// 到期仍没有 ⇒ `RecvFail::Empty`。**读空那一趟会把铃应掉**（见文件头那三拍），
    /// 故"读干"那一圈用它也不会空转、也不会丢唤醒。
    pub fn recv(&mut self, within: Wait) -> Result<M::In, RecvFail> {
        let until = crate::time::deadline(within);
        loop {
            if let Some(one) = self.try_recv()? {
                return Ok(one);
            }
            // **清必须与"我取空了"同一刻**。已清着（Busy）也是清好的状态，不是错。
            let _ = self.bell.hush();
            // 应铃之后再探一次：写端恰在这两拍之间落的那一格不会丢。
            if let Some(one) = self.try_recv()? {
                return Ok(one);
            }
            let remain = crate::time::remain(until);
            if remain == Wait::POLL {
                return Err(RecvFail::Empty);
            }
            // 等铃：**它只是"有事"**，醒来还得回环里读（可能已被别人读干，或读到的还是
            // 那些丢过的格）。故这里不把铃当成"有一条"的承诺。
            if !self
                .bell
                .wait(remain)
                .map_err(|e| RecvFail::Mail(e.source))?
            {
                return Err(RecvFail::Empty);
            }
        }
    }

    /// 本端**跳过**的枚数（写端顶掉未读格 ⇒ 我见格号跳了；从构造起累计）。
    ///
    /// 调用方按前后差值判"到手的这一条之前丢没丢"。
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// 页里那枚 `lost`（**写端**顶掉未读格的条数——与 [`Reader::skipped`] 分开记，不记两遍）。
    pub fn lost(&self) -> u64 {
        lost(self.ring)
    }

    /// 本端那一枚铃的号。
    pub fn source(&self) -> env::Source {
        self.bell.source()
    }

    pub fn bell(&self) -> PieToken {
        self.bell.token()
    }
}

/// 收不回来：三格**分得开**（与 `hand::RecvFail` 同一套词，只多"这一刻没读到"那一格）。
#[derive(Debug)]
pub enum RecvFail {
    /// 期限内没有可取之事（`Wait::POLL` 那一档就是"探测一次"）。
    Empty,
    /// 载体那一层搬不动（`Busy` / `Dead` / `Denied` / `Gone`）。
    Mail(MailFail),
    /// 收到了 `len` 字节，解不动（或那一格放不下它——这两种都由 `M` 那一族自己判）。
    Unread(usize),
}
