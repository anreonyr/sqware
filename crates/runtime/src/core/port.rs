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

use env::{EnvError, EnvResult, PieToken, TaskId, make_err};

use crate::env::mail::{self, AnyPie, HolePie};

/// D1 负码：无权 / 协议错（与 `crates/protocol` 各协议的负码同表）。
fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(-1))
}

// ── 两族视图：**搬家后的名字照旧** ─────────────────────────
//
// `Access` / `Policy` 现在住 `env`（与 `Permission` 同层，见 `env::permission`）——
// 它们只认 `Permission`，一处也不碰内核，而住在这一层会让 `protocol` 那一份
// （`driver::supply::frame`，荷载每一格都带着它们）上不了宿主靶。
// 这里把名字**转出去**：下面 `ship` 的签名与**全部调用点**（22 个文件里的
// `runtime::core::port::{Access, Policy}`）都照旧。
pub use env::{Access, Policy};

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
/// - `HandedOver`  — 源枚**已经交出去过**（一枚门闩至多一个 heir）——不是失败，交回即复原
/// - `OoM`    — 对端表备不出容量（锚已回滚：等于没交出过）
///
/// 四个码都不折平（内核 `gate::accord` 的判决原样过线）：旧注把"已被关住"写在
/// `Denied` 那一行——**照实记：那是错的**，关住的码是 `HandedOver`(-7)。
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
        let reply = HolePie::unseal(env::Mark::of("back"))?;
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
