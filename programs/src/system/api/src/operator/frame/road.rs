//! Status/Tally/Word）与应答聚合 Union、Said。

use alloc::string::String;
use env::{Mark, PieToken, TaskId};

use crate::operator::path::{Path, PathBuf};

use wire::message::Message;

use super::vocab::{BAD, EntryId, PANE_CAP, Where};
use env::wire::Span as _;
use wire::OK;

/// 问话缓冲上限：为带名称的 namespace 与 Entry 相对路径留足 512 字节。
pub const REQ_LEN: usize = 512;

/// 一答的**上限**：列表答形最大（`[status][条数][号…]`）。一条 `Pane` 本来就不超过
/// PANE_CAP 枚 ⇒ **一趟答得完，没有"未完"那一格**（对照 `coalition` 那一侧：盟籍
/// 没有上限，故那里必须带一格"未完"）
/// 本族那只缓冲就是它（Message::Buf）；其他答形都短于它。
pub const UNION_LEN: usize = 2 + PANE_CAP * 8;

const _: () = assert!(NamespaceFrame::LEN <= REQ_LEN);
const _: () = assert!(WatchFromFrame::LEN <= REQ_LEN);
const _: () = assert!(ReferenceFrame::LEN <= UNION_LEN);
const _: () = assert!(WatchPathFrame::LEN <= UNION_LEN);

const _: () = assert!(Status::LEN + 31 <= UNION_LEN);

const _: () = assert!(Status::LEN + <[u8; 8] as env::wire::Field>::WIDTH <= UNION_LEN);

#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
pub struct RoadFrame {
    pub op: u8,
    pub road: PathBuf,
}

const _: () = assert!(RoadFrame::LEN == 1 + Path::LEN);

/// `List` 那一问：动作码 ＋ 容器坐标
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct List {
    pub op: u8,
    pub at: Where,
}

/// `Find` / `Name` 共用的形状：动作码 ＋ 一枚号
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    pub op: u8,
    pub id: EntryId,
}

/// `watch` 那一问：动作码 ＋ 要订的那条路 ＋ **订阅者那一枚孔**（事件往哪儿递）
///
/// 孔由订阅者**自己**铸、经这条问话交过来（与 `land` 交一枚入口同一条手）。树上**只记不换**
/// ——撤订靠那一枚孔作废（订阅者退场即失效），故没有第二条形状。
///
/// **它从前交两块（页 ＋ 铃）**：那时事件由持树者直接写进订阅者那一页。改成"递手"之后
/// 跨域只剩这一枚孔（字节仍住持树者自己那一具架里，取走那一刻内核复制一次）。
#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
pub struct WatchFrame {
    pub op: u8,
    pub road: PathBuf,
    pub hole: PieToken,
}

#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
pub struct WatchFromFrame {
    pub op: u8,
    pub root: u8,
    pub operator: TaskId,
    pub base: EntryId,
    pub road: PathBuf,
    pub hole: PieToken,
}

#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
#[frame(len = 512)]
pub struct NamespaceFrame {
    pub op: u8,
    pub mark: u64,
    pub name: String,
}

#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
pub struct RelativeFrame {
    pub op: u8,
    pub root: u8,
    pub operator: TaskId,
    pub base: EntryId,
    pub road: PathBuf,
}

#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReferenceFrame {
    pub status: u8,
    pub operator: TaskId,
    pub root: u8,
    pub entry: EntryId,
}

#[derive(env::Frame, Clone, PartialEq, Eq, Debug)]
pub struct WatchPathFrame {
    pub status: u8,
    pub road: PathBuf,
}

/// **一问的荷载**——一个动作一条形状，没有"报法"那一格可以填错
/// 号那一侧全按 EntryId 走；名字只出现在两条路上：Req::Road（`seek` 收的那条路）
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Req {
    Road(PathBuf),
    /// `list`：列那一块 `Pane` 里的号
    List(Where),
    /// `find`：那一号后面那一枚 Pie
    Find(EntryId),
    /// `name`：那一号此刻叫什么
    Name(EntryId),
    /// `watch`：订 `road` 这条子树；此后树上真变了就往 `hole` 那一枚孔上**递一只手**
    /// （字节住持树者那一具架里，取走时复制一次）
    Watch {
        road: PathBuf,
        hole: PieToken,
    },
    WatchFrom {
        base: super::super::Entry,
        road: PathBuf,
        hole: PieToken,
    },
    Resolve {
        mark: Mark,
        name: String,
    },
    Relative {
        base: super::super::Entry,
        road: PathBuf,
    },
}

/// 一帧「列」的读数：号最多 PANE_CAP 枚
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Listing {
    ids: [EntryId; PANE_CAP],
    n: usize,
}

impl Listing {
    /// 空的那一串
    pub const fn new() -> Listing {
        Listing {
            ids: [EntryId::new(0); PANE_CAP],
            n: 0,
        }
    }

    /// 收一串（**收够 PANE_CAP 枚就停**：一条 pane 本来就不超过它）
    pub fn of(ids: impl Iterator<Item = EntryId>) -> Listing {
        let mut listing = Listing::new();
        for id in ids.take(PANE_CAP) {
            listing.push(id);
        }
        listing
    }

    /// 按号序（就是帧里的次序）走一遍
    pub fn iter(&self) -> impl Iterator<Item = EntryId> + '_ {
        self.ids[..self.n].iter().copied()
    }

    /// 那一段号——**编那一侧要它**（`store_tail` 走的是一条切片，不是一个迭代器）
    pub fn as_slice(&self) -> &[EntryId] {
        &self.ids[..self.n]
    }

    /// 收一枚。**满了就丢**：一条 pane 本来就不超过 PANE_CAP 枚
    fn push(&mut self, id: EntryId) {
        if let Some(slot) = self.ids.get_mut(self.n) {
            *slot = id;
            self.n += 1;
        }
    }
}

/// **头一格**：状态。它自己就是"一格状态"那一形（六格失败与"门外那两格"都走它），也是另外
/// 三形的起头
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Status {
    pub status: u8,
}

/// 「列」那一形的**头两格**：状态 ＋ **条数**（后面跟着那么多个号——那是尾巴，走
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tally {
    pub status: u8,
    pub count: u8,
}

/// 「号」那一形：`[status][8 字节]`——**定长 9**（`part` / `seek` 答坐标、`find` 答门闩
/// 线上逐字同形）
#[derive(env::Frame, Clone, Copy, PartialEq, Eq, Debug)]
pub struct Word {
    pub status: u8,
    pub word: [u8; 8],
}

/// **一答的形状**——状态、列表、名字、树内编号、完整 Entry 或资源句柄。
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Union {
    /// 一格状态（成功 / 六格失败 / 门外那两格）——**没有任何荷载**
    Status(u8),
    /// `list` 的下场：一串号
    List(Listing),
    /// `name` 的下场：一枚名字（**长度即名长**）
    Name(String),
    /// `part` / `seek` 的下场：那一格**坐标**
    Entry(EntryId),
    Reference(super::super::Entry),
    WatchPath(PathBuf),
    /// `find` 的下场：那一格是"我给你的那一枚**在你表里**是几号"（PieToken）
    /// **与 Union::Entry 同形不同物**（都是 `[OK][8 字节]`）而**另起一格、不复用**：两枚号
    /// 类型不同，混用就是把"树的坐标"与"你表里的门闩"当成一件事
    Seed(PieToken),
}

/// **收进来的一答**：**原样的字节** ＋ 四个读法
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Said {
    buf: [u8; UNION_LEN],
    len: usize,
}

impl Said {
    /// 这一条答的字节
    fn bytes(&self) -> &[u8] {
        self.buf.get(..self.len).unwrap_or(&[])
    }

    /// 那一格状态（四种答形的头一格都是它）
    pub fn code(&self) -> u8 {
        self.bytes().first().copied().unwrap_or(BAD)
    }

    /// A complete known status response, without a payload.
    pub fn status(&self) -> Option<u8> {
        let code = self.code();
        (self.bytes().len() == Status::LEN
            && (code == OK || code == BAD || super::vocab::code_to_fail(code).is_some()))
        .then_some(code)
    }

    /// Only a complete failure status establishes that an operation was rejected.
    pub fn failure_status(&self) -> Option<u8> {
        self.status().filter(|code| *code != OK)
    }

    /// 按「号」那一形读（`land` / `part` / `seek` 的下场）：`[status][8 字节]` → **坐标**
    /// 状态不是 OK ⇒ `Err(那一格码)`；不是那一形（长度不对）⇒ `Err(BAD)`
    pub fn entry(&self) -> Result<EntryId, u8> {
        Ok(EntryId::from_bytes(self.word()?))
    }

    pub fn watch_path(&self) -> Result<PathBuf, u8> {
        if self.code() != OK {
            return Err(self.code());
        }
        let (frame, end) = WatchPathFrame::fetch_at(self.bytes(), 0).ok_or(BAD)?;
        if end != self.bytes().len() {
            return Err(BAD);
        }
        Ok(frame.road)
    }

    pub fn reference(&self) -> Result<super::super::Entry, u8> {
        if self.code() != OK {
            return Err(self.code());
        }
        let bytes = self.bytes();
        if bytes.len() != ReferenceFrame::LEN {
            return Err(BAD);
        }
        let frame = ReferenceFrame::fetch_at(bytes, 0).ok_or(BAD)?.0;
        if frame.root > 1 || (frame.root == 1 && frame.entry != EntryId::new(0)) {
            return Err(BAD);
        }
        Ok(super::super::Entry {
            operator: frame.operator,
            target: if frame.root == 1 {
                super::super::EntryTarget::Root
            } else {
                super::super::EntryTarget::Node(frame.entry)
            },
        })
    }

    /// 按「门闩」那一形读（`find` 的下场）：同一形状 → **你表里的那一枚号**
    pub fn seed(&self) -> Result<PieToken, u8> {
        PieToken::from_bytes(&self.word()?).ok_or(BAD)
    }

    /// 「号」那一形里的那 8 字节（上面两个读法共用的那一格）
    fn word(&self) -> Result<[u8; 8], u8> {
        let code = self.code();
        if code != OK {
            return Err(code);
        }
        let bytes = self.bytes();
        if bytes.len() != Word::LEN {
            return Err(BAD);
        }
        Ok(Word::fetch_at(bytes, 0).ok_or(BAD)?.0.word)
    }

    /// 按「名」那一形读（`name` 的下场）：`[status][名字]` → 一枚名字
    /// 名字读不懂（空 / 太长 / 含 NUL / 不是 UTF-8）⇒ `Err(BAD)`：那一侧旧日的四格失败域
    pub fn name(&self) -> Result<String, u8> {
        let code = self.code();
        if code != OK {
            return Err(code);
        }
        let bytes = self.bytes();
        let text = env::wire::fetch_bytes(bytes, Status::LEN).ok_or(BAD)?;
        Ok(String::from(core::str::from_utf8(text).map_err(|_| BAD)?))
    }

    /// 按「列」那一形读（`list` 的下场）：`[status][条数][号…]` → 一串号
    /// **帧长即条数**：条数与剩下那些字节对不上（或条数超过 PANE_CAP）⇒
    /// `Err(BAD)`——短一字节也是它
    pub fn list(&self) -> Result<Listing, u8> {
        let code = self.code();
        if code != OK {
            return Err(code);
        }
        let bytes = self.bytes();
        let head = Tally::fetch_at(bytes, 0).ok_or(BAD)?.0;
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
    type Buf = [u8; UNION_LEN];
    const EMPTY: Self::Buf = [0u8; UNION_LEN];

    /// 编进 `out`：状态由形状给（不在别处再写一遍），变长那两段交给 env::wire 的两个尾巴
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
            Union::WatchPath(road) => WatchPathFrame {
                status: OK,
                road: road.clone(),
            }
            .store_at(out, 0),
            Union::Reference(entry) => ReferenceFrame {
                status: OK,
                operator: entry.operator,
                root: u8::from(entry.target == super::super::EntryTarget::Root),
                entry: match entry.target {
                    super::super::EntryTarget::Root => EntryId::new(0),
                    super::super::EntryTarget::Node(id) => id,
                },
            }
            .store_at(out, 0),
            Union::Seed(seed) => Word {
                status: OK,
                word: seed.to_bytes(),
            }
            .store_at(out, 0),
        }
    }

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
// 运行期——它是同一棵树的两半。两形的总说明与 `Tip` / `TipIn` 在下面。（"一格号"那一形

// 两形走**同一个洞、同一个读者**（提示之路 = 装配侧 → 持树者）：既不经过会话、也没有客人
// ——"往树上立一路"由持树者在自己核里做（`programs/src/system/operator/plate.rs::plate`）。
// **首格 `kind` 说这一帧是哪一形**——与客人那一族的动作码同一条纪律：一个动作一条形状，
