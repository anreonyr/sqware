//! operator::frame 的**客人那一面**：`RoadFrame` 一轮七手 · `Req` 那一形 · 回话（`Listing`/
//! `Status`/`Tally`/`Word`）与应答聚合 `Union`、`Said`。

use alloc::string::String;
use env::{PieToken };

use crate::common::path::{Path, PathBuf};

use crate::wire::id::Id as _;
use crate::wire::message::Message;

use super::vocab::{BAD, EntryId, PANE_CAP, Permit, Where};
use crate::wire::fail_codes::OK;

/// 问话那一侧的上界：**最长那一条**（`Road`：`op` ＋ [`Path::LEN`]）。
/// 服务端按它备一只缓冲（收下来的帧不会超过它），各条问话的**实际**长度由形状说——定长那几条
/// 是字段表求和（`LEN`），`Road` 那一格是 [`RoadFrame::store_at`] 交回的游标。
pub const REQ_LEN: usize = RoadFrame::LEN;

/// 一答的**上限**：四种答形里最大的那一形（`[status][条数][号…]`）。一条 `Pane` 本来就不超过
/// [`PANE_CAP`] 枚 ⇒ **一趟答得完，没有"未完"那一格**（对照 `coalition` 那一侧：盟籍
/// 没有上限，故那里必须带一格"未完"）。
/// 本族那只缓冲就是它（[`Message::Buf`]）；另两形都短于它——编译期钉住（`名` 那一形最长是
/// 状态 ＋ 名字那一格的上界（31 字节），`号` 那一形是状态 ＋ 8）。
pub const UNION_LEN: usize = 2 + PANE_CAP * 8;

// 31 = 名字那一格在**这一族**里的上界（长度那一字节不在这一形里：长度即内容）。

const _: () = assert!(Status::LEN + 31 <= UNION_LEN);

const _: () = assert!(Status::LEN + <[u8; 8] as env::wire::Field>::WIDTH <= UNION_LEN);

#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
pub struct RoadFrame {
    pub op: u8,
    pub road: PathBuf,
}

const _: () = assert!(RoadFrame::LEN == 1 + Path::LEN);

/// `List` 那一问：动作码 ＋ 容器坐标。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct List {
    pub op: u8,
    pub at: Where,
}

/// `Part` 那一问：动作码 ＋ 容器坐标 ＋ 新名。
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
#[frame(len = 42)]
pub struct Part {
    pub op: u8,
    pub at: Where,
    pub name: String,
}

#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
#[frame(len = 60)]
pub struct Land {
    pub op: u8,
    pub at: Where,
    pub name: String,
    pub entry: PieToken,
    pub mine: bool,
    pub permit: Permit,
}

/// `Find` / `Trim` / `Name` 那三问**共用**的形状：动作码 ＋ 一枚号。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    pub op: u8,
    pub id: EntryId,
}

/// **一问的荷载**——一个动作一条形状，没有"报法"那一格可以填错。
/// 号那一侧全按 [`EntryId`] 走；名字只出现在两条路上：[`Req::Road`]（`seek` 收的那条路）
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Req {
    Road(PathBuf),
    /// `list`：列那一块 `Pane` 里的号。
    List(Where),
    /// `part`：在那一块 `Pane` 下，给这个新名分一格。
    Part {
        at: Where,
        name: String,
    },
    /// `land`：在那一块 `Pane` 下，给这个新名落一枚。
    /// `entry` 是**经会话交出去之后**、种在持树者表里的那一个号（`ship` 换回来的），
    /// 不是"客人的 Pie 是几号"——两个编号空间不同源。
    Land {
        at: Where,
        name: String,
        entry: PieToken,
        permit: Permit,
        mine: bool,
    },
    /// `find`：那一号后面那一枚 Pie。
    Find(EntryId),
    /// `trim`：把那一号剪掉。
    Trim(EntryId),
    /// `name`：那一号此刻叫什么。
    Name(EntryId),
}

/// 一帧「列」的读数：号最多 [`PANE_CAP`] 枚。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Listing {
    ids: [EntryId; PANE_CAP],
    n: usize,
}

impl Listing {
    /// 空的那一串。
    pub const fn new() -> Listing {
        Listing {
            ids: [EntryId::new(0); PANE_CAP],
            n: 0,
        }
    }

    /// 收一串（**收够 [`PANE_CAP`] 枚就停**：一条 pane 本来就不超过它）。
    pub fn of(ids: impl Iterator<Item = EntryId>) -> Listing {
        let mut listing = Listing::new();
        for id in ids.take(PANE_CAP) {
            listing.push(id);
        }
        listing
    }

    /// 按号序（就是帧里的次序）走一遍。
    pub fn iter(&self) -> impl Iterator<Item = EntryId> + '_ {
        self.ids[..self.n].iter().copied()
    }

    /// 那一段号——**编那一侧要它**（`store_tail` 走的是一条切片，不是一个迭代器）。
    pub fn as_slice(&self) -> &[EntryId] {
        &self.ids[..self.n]
    }

    /// 收一枚。**满了就丢**：一条 pane 本来就不超过 [`PANE_CAP`] 枚。
    fn push(&mut self, id: EntryId) {
        if let Some(slot) = self.ids.get_mut(self.n) {
            *slot = id;
            self.n += 1;
        }
    }
}

/// **头一格**：状态。它自己就是"一格状态"那一形（六格失败与"门外那两格"都走它），也是另外
/// 三形的起头。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Status {
    pub status: u8,
}

/// 「列」那一形的**头两格**：状态 ＋ **条数**（后面跟着那么多个号——那是尾巴，走
/// [`env::wire::store_tail`]）。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tally {
    pub status: u8,
    pub count: u8,
}

/// 「号」那一形：`[status][8 字节]`——**定长 9**（`part` / `seek` 答坐标、`find` 答门闩，
/// 线上逐字同形）。
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Word {
    pub status: u8,
    pub word: [u8; 8],
}

/// **一答的形状**——答有四种：一格状态 / 一串号 / 一枚名字 / 一枚号。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Union {
    /// 一格状态（成功 / 六格失败 / 门外那两格）——**没有任何荷载**。
    Status(u8),
    /// `list` 的下场：一串号。
    List(Listing),
    /// `name` 的下场：一枚名字（**长度即名长**）。
    Name(String),
    /// `part` / `seek` 的下场：那一格**坐标**。
    Entry(EntryId),
    /// `find` 的下场：那一格是"我给你的那一枚**在你表里**是几号"（[`PieToken`]）。
    /// **与 [`Union::Entry`] 同形不同物**（都是 `[OK][8 字节]`）而**另起一格、不复用**：两枚号
    /// 类型不同，混用就是把"树的坐标"与"你表里的门闩"当成一件事。
    Seed(PieToken),
}

/// **收进来的一答**：**原样的字节** ＋ 四个读法。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Said {
    buf: [u8; UNION_LEN],
    len: usize,
}

impl Said {
    /// 这一条答的字节。
    fn bytes(&self) -> &[u8] {
        self.buf.get(..self.len).unwrap_or(&[])
    }

    /// 那一格状态（四种答形的头一格都是它）。
    pub fn code(&self) -> u8 {
        self.bytes().first().copied().unwrap_or(BAD)
    }

    /// 按「号」那一形读（`land` / `part` / `seek` 的下场）：`[status][8 字节]` → **坐标**。
    /// 状态不是 [`OK`] ⇒ `Err(那一格码)`；不是那一形（长度不对）⇒ `Err(BAD)`。
    pub fn entry(&self) -> Result<EntryId, u8> {
        Ok(EntryId::from_bytes(self.word()?))
    }

    /// 按「门闩」那一形读（`find` 的下场）：同一形状 → **你表里的那一枚号**。
    pub fn seed(&self) -> Result<PieToken, u8> {
        PieToken::from_bytes(&self.word()?).ok_or(BAD)
    }

    /// 「号」那一形里的那 8 字节（上面两个读法共用的那一格）。
    fn word(&self) -> Result<[u8; 8], u8> {
        let code = self.code();
        if code != OK {
            return Err(code);
        }
        let bytes = self.bytes();
        if bytes.len() != Word::LEN {
            return Err(BAD);
        }
        Ok(Word::fetch(bytes).ok_or(BAD)?.word)
    }

    /// 按「名」那一形读（`name` 的下场）：`[status][名字]` → 一枚名字。
    /// 名字读不懂（空 / 太长 / 含 NUL / 不是 UTF-8）⇒ `Err(BAD)`：那一侧旧日的四格失败域
    /// 在这里**归一格**——问的人能做的补救是同一件（这一帧坏了，重问）。
    pub fn name(&self) -> Result<String, u8> {
        let code = self.code();
        if code != OK {
            return Err(code);
        }
        let bytes = self.bytes();
        let text = env::wire::fetch_bytes(bytes, Status::LEN).ok_or(BAD)?;
        Ok(String::from(core::str::from_utf8(text).map_err(|_| BAD)?))
    }

    /// 按「列」那一形读（`list` 的下场）：`[status][条数][号…]` → 一串号。
    /// **帧长即条数**：条数与剩下那些字节对不上（或条数超过 [`PANE_CAP`]）⇒
    /// `Err(BAD)`——短一字节也是它。
    pub fn list(&self) -> Result<Listing, u8> {
        let code = self.code();
        if code != OK {
            return Err(code);
        }
        let bytes = self.bytes();
        let head = Tally::fetch(bytes).ok_or(BAD)?;
        let count = head.count as usize;
        if count > PANE_CAP {
            return Err(BAD);
        }
        let body = bytes.get(Tally::LEN..).ok_or(BAD)?;
        let mut ids = [EntryId::new(0); PANE_CAP];
        let end = env::wire::fetch_tail(body, 0, &mut ids[..count]).ok_or(BAD)?;
        if end != body.len() {
            return Err(BAD);
        }
        Ok(Listing::of(ids[..count].iter().copied()))
    }
}

impl Message for Union {
    type In = Said;
    /// 这一族的缓冲：**最大那一形**（[`UNION_LEN`]）。
    type Buf = [u8; UNION_LEN];
    const EMPTY: Self::Buf = [0u8; UNION_LEN];

    /// 编进 `out`：状态由形状给（不在别处再写一遍），变长那两段交给 `env::wire` 的两个尾巴。
    fn store(&self, out: &mut [u8]) -> Option<usize> {
        match self {
            Union::Status(code) => Status { status: *code }.store_at(out, 0),
            Union::List(list) => {
                let ids = list.as_slice();
                let head = Tally {
                    status: OK,
                    count: ids.len() as u8,
                };
                head.store_at(out, 0)?;
                env::wire::store_tail(out, Tally::LEN, ids)
            }
            Union::Name(name) => {
                let at = Status { status: OK }.store_at(out, 0)?;
                // 名长即这一帧剩下的那些字节（**长度即内容**那一形）。
                env::wire::store_bytes(out, at, name.as_bytes())
            }
            Union::Entry(id) => Word {
                status: OK,
                word: id.to_bytes(),
            }
            .store_at(out, 0),
            Union::Seed(seed) => Word {
                status: OK,
                word: seed.to_bytes(),
            }
            .store_at(out, 0),
        }
    }

    /// 收一条：**原样收下**（空帧、或长过这一族的缓冲 ⇒ `None`）。形状不在这里判——
    fn fetch(bytes: &[u8]) -> Option<Said> {
        if bytes.is_empty() {
            return None;
        }
        let mut buf = [0u8; UNION_LEN];
        buf.get_mut(..bytes.len())?.copy_from_slice(bytes);
        Some(Said {
            buf,
            len: bytes.len(),
        })
    }
}

// 它不在上面那张图里：上面那几帧是**客人 ↔ 持树者**的一问一答，这几条是**装配者递过来
// 的东西**（立一条路 / 一位客人）。两族同住本文件，因为"帧形只有一处"这一条不分装配期与
// 运行期——它是同一棵树的两半。两形的总说明与 `Tip` / `TipIn` 在下面。（"一格号"那一形

// 两形走**同一个洞、同一个读者**（提示之路 = 装配侧 → 持树者）：既不经过会话、也没有客人
// ——"往树上立一路"由持树者在自己核里做（`programs/src/system/operator/plate.rs::plate`）。
// **首格 `kind` 说这一帧是哪一形**——与客人那一族的动作码同一条纪律：一个动作一条形状，
