//! port — Hole 的通讯协议：授出（`ship`）、坐标（`To`）、一条会话（`Port`）。
//!
//! 一个类型回答一个问题，一个也不多：
//!
//! ```text
//! Access / Policy   对端能做什么 / 这一枚能怎么流动（四位分两族，混族不可表达）
//! ship              授出：子集由两族拼出，空集本地拒（不发 envcall）
//! To                对端坐标：`peer`（那**张表**的主人）与 `seed`（种在**它表里**的那一枚）
//! Port              两枚孔的配对（往哪推 / 从哪收 / 对面是谁）+ 收发关
//! ```
//!
//! **本层不是内核对象**：槽在内核 `mail`、权柄判定与级联在内核 `gate`、报文布局与
//! 一次往返在 `crates/protocol`，这里只有"怎么用"。
//!
//! **`Port` 不加新动词**：`open` / `push` / `pull` / `shut` 与 `HolePie` 那一族同名同形，
//! 差别只在三件事——推的是哪一枚、收的是哪一枚、收的时候**校不校来源**。编帧、解帧、
//! 一问一答的时序、开会话的握手都不在这里：那些属于协议（见 `crates/protocol`）。

use core::ops::{BitAnd, BitOr, Not};

use env::{EnvError, EnvResult, Permission, PieToken, TaskId, make_err};

use crate::env::mail::{self, AnyPie, HolePie};

/// D1 负码：无权 / 协议错（与 `crates/protocol` 各协议的负码同表）。
fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(-1))
}

/// 读写族：对端对这份资源**能做什么**。
///
/// 与 [`Policy`] 分成两个类型是刻意的：合成一个 `Permission` 时，调用方可以把
/// 形态位写进"读写"该在的位置，而两族相乘的十六格里有一格是空集、一格只有形态。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Access(Permission);

/// 读写族那两位（解线上权时用来挡混族）。
const ACCESS_MASK: Permission = Permission::FETCH.union(Permission::STORE);
/// 传递族那两位（同上）。
const POLICY_MASK: Permission = Permission::VEST.union(Permission::ONLY);

impl Access {
    /// 什么都没有：两族都取 `NONE` 才是空集（那种授予 `ship` 就地拒）。
    pub const NONE: Access = Access(Permission::empty());
    /// 取用 / 观察 / 接收（`pull` / `hush` / `open` / `Await`）。
    pub const FETCH: Access = Access(Permission::FETCH);
    /// 投递 / 修改（`push` / `ring` / `hang`）。
    pub const STORE: Access = Access(Permission::STORE);
    /// 取与投。**只为常量表而设**：`|` 不是 `const fn`，而需求单（`programs::needs`）
    /// 是编译期常量表——两族各一位，合成走这里。
    pub const FETCH_STORE: Access = Access(Permission::FETCH.union(Permission::STORE));

    /// 位（交给 `Accord` 的那一半）。
    pub const fn bits(self) -> Permission {
        self.0
    }

    /// 从线上那几位解回（**只收本族的位**，混族与未知位一律拒）。
    ///
    /// 与"调用点写不出裸子集"是同一条：这条入口只认本来就是 `Access` 造出来的值，
    /// 故它不构成第二条授权路径，只给"过线的权"一个回来的门。
    pub const fn from_bits(bits: u32) -> Option<Access> {
        match Permission::from_bits(bits) {
            Some(p) if p.bits() & ACCESS_MASK.bits() != p.bits() => None,
            Some(p) => Some(Access(p)),
            None => None,
        }
    }
}

impl BitOr for Access {
    type Output = Access;

    fn bitor(self, rhs: Access) -> Access {
        Access(self.0 | rhs.0)
    }
}

/// 传递族：这一枚**能怎么流动**。
///
/// 四格读作四件事——**后两格只有独占资源上才成立**（源枚带 `ONLY` 才给得出）：
///
/// ```text
/// NONE         分  双方各持一份，对端不能再授出
/// VEST         借  双方各持一份，对端可以再授出
/// ONLY         让  我失去这一枚，对端不能再授出
/// VEST | ONLY  给  我失去这一枚，对端可以再授出
/// ```
///
/// 这四格里，**只有 `VEST` 一位是调用方的选择**：形态（复制还是移交）由**源枚**定
/// ——`ONLY` 是资源事实，`Accord` 校验 `subset` 与源枚一致，不一致 ⇒ 拒。
/// 所以想复制一枚独占资源，在这里就发不出去。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Policy(Permission);

impl Policy {
    /// 不能再授出（源枚不动）。
    pub const NONE: Policy = Policy(Permission::empty());
    /// 对端可以再授出。
    pub const VEST: Policy = Policy(Permission::VEST);
    /// 与源枚一致：独占资源的授出（这一次是**移交**，源枚在子枚存活期间不可用）。
    pub const ONLY: Policy = Policy(Permission::ONLY);

    /// 位（交给 `Accord` 的那一半）。
    pub const fn bits(self) -> Permission {
        self.0
    }

    /// 从线上那几位解回（**只收传递族那两位**）——与 [`Access::from_bits`] 同一条口径。
    pub const fn from_bits(bits: u32) -> Option<Policy> {
        match Permission::from_bits(bits) {
            Some(p) if p.bits() & POLICY_MASK.bits() != p.bits() => None,
            Some(p) => Some(Policy(p)),
            None => None,
        }
    }
}

impl BitOr for Policy {
    type Output = Policy;

    fn bitor(self, rhs: Policy) -> Policy {
        Policy(self.0 | rhs.0)
    }
}

impl BitAnd for Policy {
    type Output = Policy;

    fn bitand(self, rhs: Policy) -> Policy {
        Policy(self.0 & rhs.0)
    }
}

/// 取反是**为"剔掉某一位"而开的口**：`policy & !Policy::VEST` 读起来就是
/// "形态照旧，只去掉'能再授出'那一位"——调用点不必为一个动作另起名词。
///
/// 取反之后的值可能带上别的族的位（如 `FETCH`），故它**只配与 `&` 联用**：
/// 单独拿它去 `ship` 就是把两族人造的值当形态发出去。
impl Not for Policy {
    type Output = Policy;

    fn not(self) -> Policy {
        Policy(!self.0)
    }
}

/// 对端坐标：**两半成对**——`peer`（那**张表**的主人）与 `seed`（种在**它表里**的
/// 那一枚）。
///
/// **为什么必须成对**：`seed` 是个号，号只在那一张表里有意义——离开 `peer` 就只是一
/// 个数（旧 `Channel` 只存 `at_peer` 的病根）。而本仓其余"种出去的号"（`Quay` /
/// `Pier` / `Referred`）的对端由**方向**给出（子→父 / 父→子 / 一问一答），唯有这里是
/// **回信孔**：谁都能往它推，方向推不出对端 ⇒ 两半只能一起带着。
///
/// 字段私有、只有 [`ship`] 能造。
#[derive(Clone, Copy, Debug)]
pub struct To {
    peer: TaskId,
    seed: PieToken,
}

impl To {
    fn new(peer: TaskId, seed: PieToken) -> To {
        To { peer, seed }
    }

    /// 那**张表**的主人——`pull` 拿它核对"这条回复是不是它推的"。
    pub const fn peer(self) -> TaskId {
        self.peer
    }

    /// 种在**它表里**的那一枚——写进帧，对端按它 `push`。
    pub const fn seed(self) -> PieToken {
        self.seed
    }
}

/// 授出：把 `pie` 的一个子集授给 `peer`，返对端坐标。
///
/// **这就是"授出"的全部机制**：子集由两族拼出，混族写不出来；空集本地拒（不发
/// envcall）。调用点从此写不出裸子集——一份子集写错就是一份多余授权，而
/// `Access`/`Policy` 让"该授什么"在签名上就说清楚了。
///
/// 泛型于 [`AnyPie`]：三种资源都有授出点（回信孔是 `HolePie`、设备门闩是
/// `PolePie`、门铃是 `NolePie`），权柄操作本就与资源种类无关。
///
/// # Errors
/// - `Denied` — 空集（本地拒）/ 源枚不持 `VEST` / 子集越界 / `ONLY` 与源枚不一致 /
///   对端不存在
/// - `Dead`   — 源枚的资源已封印
/// - `Caged`  — 源枚**已经交出去过**（一枚门闩至多一个 heir）——不是失败，交回即复原
/// - `OoM`    — 对端表备不出容量（锚已回滚：等于没交出过）
///
/// 四个码都不折平（内核 `gate::accord` 的判决原样过线）：旧注把"已被关住"写在
/// `Denied` 那一行——**照实记：那是错的**，关住的码是 `Caged`(-7)。
pub fn ship<P: AnyPie>(pie: &P, peer: TaskId, access: Access, policy: Policy) -> EnvResult<To> {
    let subset = access.bits() | policy.bits();
    if subset.is_empty() {
        return Err(denied());
    }
    let seed = pie.accord(peer, subset)?;
    Ok(To::new(peer, seed))
}

/// 一条会话：入口门闩（借入）+ 回信孔（自有）+ 对端坐标。
///
/// `entry` 的所有权**不在本类型**——只借入，`shut` 不碰它：今天 `Service` 释放
/// entry、`Console` 不释放，那是**策略**，不进结构。
pub struct Port {
    to: To,
    entry: HolePie,
    reply: HolePie,
}

impl Port {
    /// 开一条会话：`entry` = 对端入口门闩在**我这一份**表里的句柄。
    ///
    /// 做三件事：问出对端是谁（`Reserve(entry).owner`——`vestor` 会被转发改写，
    /// `owner` 不会）、自建**回信孔**（记号 `back`：与 `guest` 借出去的那一枚同用途）、
    /// 把它的 `STORE` 授给对端。回信孔只要 `STORE`：对端往它推，读的一侧是我。
    ///
    /// # Errors
    /// - `Denied` — 入口门闩不在表里 / 不是孔（记号为孔所独有）/ 无开辟者
    ///   （`owner == 0`，引导期那批设备门闩）/ 交不出去
    /// - `Dead`   — 入口那一枚的资源已封印
    /// - `OoM`    — 本端或对端那张表备不下这一枚
    pub fn open(entry: &HolePie) -> EnvResult<Port> {
        let peer = mail::reserve(entry.token())?.1;
        if peer.get() == 0 {
            return Err(denied());
        }
        let reply = HolePie::unseal("back")?;
        let to = ship(&reply, peer, Access::STORE, Policy::NONE)?;
        Ok(Port {
            to,
            entry: HolePie::from_token(entry.token()),
            reply,
        })
    }

    /// 写进帧的那一格：种在**对端表里**的那一枚。
    pub fn seed(&self) -> PieToken {
        self.to.seed()
    }

    /// 推一帧：**已编好的整帧**，本层不看内容。满则等（背压），没有上界。
    ///
    /// 成功 ≠ 对端收到：`entry` 是本端持有的一份副本，对端死了这扇门也不死。
    pub fn push(&self, frame: &[u8]) -> EnvResult<()> {
        self.entry.push(frame)
    }

    /// 收一帧：有界等 → **核对推者是不是对端** → 返恰好那一帧。
    ///
    /// 推者不符 ⇒ `Denied`，该会话应弃用（迟到的真回复仍可能落槽、污染下一次）。
    pub fn pull<'a>(&self, buf: &'a mut [u8], within: usize) -> EnvResult<&'a [u8]> {
        let (len, from) = self.reply.pull_timeout_from(buf, within)?;
        if from != self.to.peer() {
            return Err(denied());
        }
        buf.get(..len).ok_or_else(denied)
    }

    /// 关：只放下回信孔（级联已含对端那枚副本），**不碰 `entry`**。
    pub fn shut(self) -> EnvResult<()> {
        self.reply.release()
    }

    /// 借来一条会话：对端坐标 + 入口门闩 + **对端开的**回信孔。
    ///
    /// 给"回信孔由**对端**开"的协议用（`console` 的会话握手：它先要到那枚句柄，再拿它
    /// 和入口孔凑成一条会话）。与 [`Port::open`] 的差别只有这一件事——**孔的命挂谁身上**：
    /// `open` 铸的那枚命随本端（对端死了这扇门不死，`pull` 只会等到上界）；`borrow` 拿的
    /// 这枚命随对端（对端退场时内核的寿命边封印它，`pull` **当场拿到 `Dead`**）。
    pub fn borrow(peer: TaskId, entry: HolePie, reply: HolePie) -> Port {
        Port {
            to: To {
                peer,
                seed: reply.token(),
            },
            entry,
            reply,
        }
    }
}
