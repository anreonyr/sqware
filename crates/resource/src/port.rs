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
//! 一次往返在 `crates/ipc`，这里只有"怎么用"。
//!
//! **`Port` 不加新动词**：`open` / `push` / `pull` / `shut` 与 Hole 的使用面同名同形，
//! 差别只在三件事——推的是哪一枚、收的是哪一枚、收的时候**校不校来源**。编帧、解帧、
//! 一问一答的时序、开会话的握手都不在这里：那些属于协议（见 `crates/ipc`）。

use env::{
    HoleDir, MailFail, MailResult, Mark, PieFail, PieResult, PieToken, TaskId, Wait, make_fail,
};

use crate::{hole::Hole, raw};

pub use crate::reply::{Reply, ReplyError};

/// D1 负码：无权 / 协议错（与 `crates/ipc` 各协议的负码同表）。
fn denied_mail() -> erra::Error<MailFail> {
    make_fail(MailFail::Denied)
}

/// 权柄轴那一侧的"不成"（Pie 的 `Denied`）——`ship` / `open` / `shut` 用它。
fn denied_pie() -> erra::Error<PieFail> {
    make_fail(PieFail::Denied)
}

/// 访问权限与转授策略的共享 ABI 词汇。
pub use env::{Access, Policy};

/// 已识别对端的字节发送端。操作权限与存活仍由内核检查。
pub struct Sender {
    entry: Hole,
    peer: TaskId,
}

impl Sender {
    /// 导入服务入口；只借用本地能力，不承担它的释放。
    pub fn import(entry: PieToken) -> PieResult<Self> {
        let (_, peer, _) = raw::reserve(entry)?;
        if peer.get() == 0 {
            return Err(denied_pie());
        }
        Ok(Self {
            entry: Hole::from_raw(entry),
            peer,
        })
    }

    pub fn peer(&self) -> TaskId {
        self.peer
    }

    pub fn push(&self, bytes: &[u8], within: Wait) -> MailResult<()> {
        self.entry.push(bytes, within)
    }
}

/// **为什么必须成对**：`seed` 是个号，号只在那一张表里有意义——离开 `peer` 就只是一
/// 个数（旧 `Channel` 只存写端那一枚的病根）。而本仓其余"种出去的号"（协议层的
/// `Endpoint` / `Referred`）的对端由**方向**给出（子→父 / 父→子 / 一问一答），唯有这里是
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
/// 权柄操作与资源种类无关，按 token 授出。
///
/// # Errors
/// - `Denied` — 空集（本地拒）/ 源枚不持 `VEST` / 子集越界 / `ONLY` 与源枚不一致 /
///   对端不存在
/// - `Dead`   — 源枚的资源已封印
/// - `HandedOver`  — 源枚**已经交出去过**（一枚门闩至多一个 heir）——不是失败，交回即复原
/// - `OoM`    — 对端表备不出容量（锚已回滚：等于没交出过）
///
/// 四个码都不折平（内核 `gate::accord` 的判决原样过线）："已被关住"的码是
/// `HandedOver`(-7)，**不是** `Denied`——两者不要混。
pub fn ship(pie: PieToken, peer: TaskId, access: Access, policy: Policy) -> PieResult<To> {
    let subset = access.bits() | policy.bits();
    if subset.is_empty() {
        return Err(denied_pie());
    }
    // 记号**照源枚**（`Mark::NONE`）：授出这一手不改记号——记号是"哪条路"，
    // 两端认的就是同一枚（`PieCall::Accord` 那一格是给"另刻一枚"留的口）。
    let seed = env::pie::accord(pie, peer, subset, Mark::NONE)?;
    Ok(To::new(peer, seed))
}

/// 一条会话：入口门闩（借入）+ 回信孔（自有）+ 对端坐标。
///
/// `entry` 的所有权**不在本类型**——只借入，`shut` 不碰它：今天 `Service` 释放
/// entry、`Console` 不释放，那是**策略**，不进结构。
pub struct Port {
    to: To,
    entry: Hole,
    reply: Hole,
}

impl Port {
    /// 开一条会话：`entry` = 对端入口门闩在**我这一份**表里的句柄。
    ///
    /// 做三件事：问出对端是谁（`Reserve(entry).owner`——`vestor` 会被转发改写，
    /// `owner` 不会）、自建无标记回信孔、
    /// 把它的 `STORE` 授给对端。回信孔只要 `STORE`：对端往它推，读的一侧是我。
    ///
    /// # Errors
    /// - `Denied` — 入口门闩不在表里 / 不是孔（记号为孔所独有）/ 无开辟者
    ///   （`owner == 0`，引导期那批设备门闩）/ 交不出去
    /// - `Dead`   — 入口那一枚的资源已封印
    /// - `OoM`    — 本端或对端那张表备不下这一枚
    pub fn open(entry: PieToken) -> PieResult<Port> {
        let peer = raw::reserve(entry)?.1;
        if peer.get() == 0 {
            return Err(denied_pie());
        }
        let reply = Hole::unseal(Mark::NONE)?;
        let to = ship(reply.token(), peer, Access::STORE, Policy::NONE)?;
        Ok(Port {
            to,
            entry: Hole::from_raw(entry),
            reply,
        })
    }

    /// 写进帧的那一格：种在**对端表里**的那一枚。
    pub fn seed(&self) -> PieToken {
        self.to.seed()
    }

    /// **`Ok` = 那一手被取走了，不是"对端读懂了"**：`entry` 是本端持有的一份副本，对端死了这扇
    /// 门也不死（它的封印只会让这一等当场答 `Dead`）。
    pub fn push(&self, frame: &[u8]) -> MailResult<()> {
        self.entry.push(frame, Wait::Forever)?;
        self.entry.wait(HoleDir::Push, Wait::Forever).map(|_| ())
    }

    /// 收一帧：有界等 → **核对推者是不是对端** → 返恰好那一帧。
    ///
    /// 推者不符 ⇒ `Denied`，该会话应弃用（迟到的真回复仍可能落槽、污染下一次）。
    pub fn pull<'a>(&self, buf: &'a mut [u8], within: Wait) -> MailResult<&'a [u8]> {
        let (len, from) = self.reply.pull(buf, within)?;
        if from != self.to.peer() {
            return Err(denied_mail());
        }
        buf.get(..len).ok_or_else(denied_mail)
    }

    /// 关：只放下回信孔（级联已含对端那枚副本），**不碰 `entry`**。
    pub fn shut(self) -> PieResult<()> {
        env::pie::release(self.reply.token())
    }

    /// 借来一条会话：对端坐标 + 入口门闩 + **对端开的**回信孔。
    ///
    /// 给"回信孔由**对端**开"的协议用（`console` 的会话握手：它先要到那枚句柄，再拿它
    /// 和入口孔凑成一条会话）。与 [`Port::open`] 的差别只有这一件事——**孔的命挂谁身上**：
    /// `open` 铸的那枚命随本端（对端死了这扇门不死，`pull` 只会等到上界）；`borrow_raw` 拿的
    /// 这枚命随对端（对端退场时内核的寿命边封印它，`pull` **当场拿到 `Dead`**）。
    ///
    /// 这里直接导入原始 token：本方法不新增验证，也不代为释放外来资源。
    pub fn borrow_raw(peer: TaskId, entry: PieToken, reply: PieToken) -> Port {
        Port {
            to: To { peer, seed: reply },
            entry: Hole::from_raw(entry),
            reply: Hole::from_raw(reply),
        }
    }
}
