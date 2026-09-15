//! port — Hole 的通讯协议：授出（`ship`）、坐标（`To`）、一次往返（`Port`）、报文对（`Duet`）。
//!
//! 一个类型回答一个问题，一个也不多：
//!
//! ```text
//! Access / Policy   对端能做什么 / 这一枚能怎么流动（四位分两族，混族不可表达）
//! ship              授出：子集由两族拼出，空集本地拒（不发 envcall）
//! To                对端坐标：`peer`（那**张表**的主人）与 `seed`（种在**它表里**的那一枚）
//! Duet              报文对的形状：请求与回复成对绑定，布局归协议
//! Port              一枚 Hole 门闩上的一次往返：填回信地址 / push / 校来源 / 有界等 / 解码
//! ```
//!
//! **本层不是内核对象**：槽在内核 `mail`、权柄判定与级联在内核 `gate`、报文布局在
//! `crates/protocol`，这里只有"怎么用"。
//!
//! **`Port` 是 `Channel` 的替代**（旧类型已删）。它多出的三件事各对应一条教训：
//! 坐标两半成对（旧类型只存对端那半）、收尾只 `release`（级联已含对端那枚副本）、
//! 协议拿不到句柄（旧类型 `mine()` 交出整枚门闩）。
//!
//! 判据与裁决账见 `docs/port.md`。

use core::ops::BitOr;

use env::{EnvError, EnvResult, Permission, PieToken, TaskId, make_err};

use crate::env::mail::{self, AnyPie, HolePie};

/// D1 负码：无权 / 协议错（与 `crates/protocol` 各协议的负码同表）。
fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(-1))
}

/// 回信地址的线形：一枚 `PieToken` 在**对端那张表**里的编号。
pub const ADDRESS_LEN: usize = 8;

/// 回信地址在报文里的偏移——**紧跟首字段**（首字段是动词，一字节）。
///
/// 它与 [`Duet::encode`] 一起构成本层的两件套：**地址在哪儿**（这个数）、
/// **这一帧多长**（那个返回值）。两者都不由协议各自发明。
pub const ADDRESS_AT: usize = 1;

/// 从任意一条报文里抠出回信地址——**坏报文也要能抠**（拒一条请求仍欠对方一句
/// `Denied`，而那一句得知道往哪儿推）。`None` = 太短 / 全是 0（0 是本仓的"无句柄"）。
pub fn address_of(m: &[u8]) -> Option<PieToken> {
    address_at(m, ADDRESS_AT)
}

/// 把回信地址写进一条报文的**默认**地址槽（`[ADDRESS_AT..)`）。
pub fn put_address(m: &mut [u8], token: PieToken) -> Option<()> {
    put_address_at(m, ADDRESS_AT, token)
}

/// 同 [`put_address`]，但地址槽在**协议自定**的偏移上（见 [`Duet::ADDRESS_AT`]）。
pub fn put_address_at(m: &mut [u8], at: usize, token: PieToken) -> Option<()> {
    let slot = m.get_mut(at..at + ADDRESS_LEN)?;
    slot.copy_from_slice(&token.get().to_le_bytes());
    Some(())
}

/// 同 [`address_of`]，但地址槽在协议自定的偏移上。
pub fn address_at(m: &[u8], at: usize) -> Option<PieToken> {
    let slot = m.get(at..at + ADDRESS_LEN)?;
    let token = PieToken::new(usize::from_le_bytes(slot.try_into().ok()?));
    if token.get() == 0 { None } else { Some(token) }
}

/// 读写族：对端对这份资源**能做什么**。
///
/// 与 [`Policy`] 分成两个类型是刻意的：合成一个 `Permission` 时，调用方可以把
/// `CAGE` 写进"读写"该在的位置，而两族相乘的十六格里有一格是空集、一格只有形态。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Access(Permission);

impl Access {
    /// 什么都没有：两族都取 `NONE` 才是空集（那种授予 `ship` 就地拒）。
    pub const NONE: Access = Access(Permission::empty());
    /// 观察 / 接收 / 重读。
    pub const READ: Access = Access(Permission::READ);
    /// 修改 / 投递 / 写入。
    pub const WRITE: Access = Access(Permission::WRITE);

    /// 位（交给 `Accord` 的那一半）。
    pub const fn bits(self) -> Permission {
        self.0
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
/// 四个取值读作四件事：
///
/// ```text
/// NONE         分  双方各持一份，对端不能再授出
/// VEST         借  双方各持一份，对端可以再授出
/// CAGE         让  我失去这一枚，对端不能再授出
/// VEST | CAGE  给  我失去这一枚，对端可以再授出（粘性：它再授出的必带 CAGE）
/// ```
///
/// **`READ` 只可转移、不可复制**：授出 `READ` 不带 `CAGE` ⇒ 源枚仍可用 ⇒ 两个读者。
/// 单读者因此不是内核机制，是位表的推论。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Policy(Permission);

impl Policy {
    /// 不能再授出（源枚不动）。
    pub const NONE: Policy = Policy(Permission::empty());
    /// 对端可以再授出。
    pub const VEST: Policy = Policy(Permission::VEST);
    /// 一次交出：源枚在交出期间不可用。
    pub const CAGE: Policy = Policy(Permission::CAGE);

    /// 位（交给 `Accord` 的那一半）。
    pub const fn bits(self) -> Permission {
        self.0
    }
}

impl BitOr for Policy {
    type Output = Policy;

    fn bitor(self, rhs: Policy) -> Policy {
        Policy(self.0 | rhs.0)
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

    /// 对端是谁。用于校验"这条回复真是它推的"。
    pub const fn peer(self) -> TaskId {
        self.peer
    }

    /// 种在**对端表里**的那一枚——写进报文，对端按它 push。
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
/// `PolePie`、建域权是 `NolePie`），权柄操作本就与资源种类无关。
///
/// # Errors
/// - `Denied` — 空集（本地拒）/ 源枚不持 `VEST` / 子集越界 / 已被关住 / 对端不存在
/// - `Dead`   — 源枚的资源已封印
/// - `OoM`    — 对端表备不出容量
pub fn ship<P: AnyPie>(pie: &P, peer: TaskId, access: Access, policy: Policy) -> EnvResult<To> {
    let subset = access.bits() | policy.bits();
    if subset.is_empty() {
        return Err(denied());
    }
    let seed = pie.accord(peer, subset)?;
    Ok(To::new(peer, seed))
}

/// 报文对的形状：一条请求、一条回复，**成对绑定**。
///
/// 用 trait 而非闭包：闭包版可以把 A 协议的 `encode` 与 B 协议的 `decode` 一起递
/// 进去，编译器不响；`type Req` / `type Rep` 配错不可表达。
pub trait Duet {
    /// 请求载荷。
    type Req;
    /// 回复载荷。
    type Rep;
    /// 报文容器（`[u8; REQ]` 那个数组）。
    ///
    /// 为什么容器由协议自己给：稳定 Rust 里 `[u8; W::REQ]` 是泛型常量表达式
    /// （要 `generic_const_exprs`），用不了 ⇒「报文多大」这份知识只能留在实现侧。
    type Wire: AsRef<[u8]> + AsMut<[u8]>;

    /// 地址槽在本协议帧里的偏移。默认 [`ADDRESS_AT`]（紧跟首字段）；协议可覆盖——
    /// 目录协议把它声明成**帧首**：那条协议有两张线形（裸询问 / 服务调用载荷），
    /// 帧首是唯一让两者逐字相同的位置（见 `dispatch::wire` 模块头注）。
    const ADDRESS_AT: usize = ADDRESS_AT;

    /// **一块内存要多大装得下本协议任何一条帧**——给容器定容用的上界，**不是被推的
    /// 字节数**（那个数由 [`Duet::encode`] 当场报出来）。报文可短不可长：短了正是省下
    /// 的那些字节，长了接收侧装不下（`pull` 拒绝且槽不动 ⇒ 这条孔当场死住）。
    const CAP: usize;

    /// 一个空的报文容器（**请求**用）。
    fn wire() -> Self::Wire;

    /// 收回复的容器：**不能与请求那块共用**。
    ///
    /// 曾经共用过（回复收在请求帧之后的剩余空间里），短请求因此看不出问题——`Open` 只占
    /// 9 字节，尾巴上还剩得下一帧回复。而 `Write` 一类长请求把容器填到 `CAP - 8`
    /// （控制台实测 `sent=41`、`CAP=49`），`pull` 的 `max` 只剩 8 字节，回复帧（17）装不下
    /// ⇒ 内核答 `Denied` ⇒ **一次写就断一条会话**，现象是"输出少半行 + 目录查询全失败"。
    ///
    /// 定容的理由与 [`Duet::Wire`] 同款：稳定 Rust 里 `[u8; W::REP]` 用不了，故容器由协议给。
    type Reply: AsRef<[u8]> + AsMut<[u8]>;

    /// 收回复的容器（空）。
    fn reply() -> Self::Reply;

    /// 布局：把请求写进 `out`，并把**回信地址**写在 `[ADDRESS_AT..)` 上。
    ///
    /// `seed` = 种在对端表里的那一枚（[`To::seed`]）。返**本帧的字节数**——写入 `out`
    /// 的那些字节才是这一帧，其余位置一个字节都不上线（补零、保留区都由此消失）。
    /// 要不要每条请求都带地址，仍归协议说（如控制台只在 `Open` 上带）。
    fn encode(req: &Self::Req, seed: PieToken, out: &mut [u8]) -> usize;

    /// 解码回复。报文不合法 ⇒ `Denied`。
    fn decode(buf: &[u8]) -> EnvResult<Self::Rep>;
}

/// 一条会话：入口门闩（借入）+ 回信孔（自有）+ 对端坐标。
///
/// `entry` 的所有权**不在本类型**——只借入，`close` 不碰它：今天 `Service` 释放
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
    /// `owner` 不会）、自建回信孔、把它的 `WRITE` 授给对端。回信孔只要 `WRITE`：
    /// 对端往它推，读的一侧是我。
    ///
    /// # Errors
    /// - `Denied` — 入口门闩不在表里 / 无开辟者 / 授不出去
    pub fn open(entry: &HolePie) -> EnvResult<Port> {
        let peer = mail::reserve(PieToken::new(entry.token()))?.1;
        if peer.get() == 0 {
            return Err(denied());
        }
        let reply = HolePie::unseal()?;
        let to = ship(&reply, peer, Access::WRITE, Policy::NONE)?;
        Ok(Port {
            to,
            entry: HolePie::from_token(entry.token()),
            reply,
        })
    }

    /// 一次往返：填回信地址 → `push`（满则等＝背压）→ 有界等 → **校来源** → 解码。
    ///
    /// `within` = 等回复的毫秒上界（`usize::MAX` = 永久）。有上界才能把"回复被丢"
    /// 暴露成 `Busy`，而不是永久挂起；而"用户还在打字"这一类往返要的正是不设上界。
    ///
    /// **来源对不上 ⇒ `Denied`，该会话应弃用**：迟到的真回复仍可能落槽、污染下一次
    /// `call`（与 [`HolePie::pull_timeout`] 的既有契约同款）。
    pub fn call<W: Duet>(&self, req: &W::Req, within: usize) -> EnvResult<W::Rep> {
        let mut wire = W::wire();
        let sent = W::encode(req, self.to.seed(), wire.as_mut());
        self.entry
            .push(wire.as_ref().get(..sent).ok_or_else(denied)?)?;
        self.take::<W>(within)
    }

    /// **收**那一半：有界等 → 校来源 → 解码。`call` 与 [`Port::dial`] 共用。
    ///
    /// 不外放（`docs/port.md` §7 否掉"只留 `send`/`recv`"）：往返还归本类型。
    /// `dial` 借它收**首帧的答复**——那条答复由服务端建会话时推出，与本端推了哪一帧无关
    /// （故 `take` 只收，不推）。
    fn take<W: Duet>(&self, within: usize) -> EnvResult<W::Rep> {
        let mut reply = W::reply();
        let (len, from) = self.reply.pull_timeout_from(reply.as_mut(), within)?;
        if from != self.to.peer() {
            return Err(denied());
        }
        W::decode(reply.as_ref().get(..len).ok_or_else(denied)?)
    }

    /// **出话**：在对端入口孔上开一条会话，回信孔**由对端开、我借用**。
    ///
    /// 与 [`Port::open`] 的差别只有一件事——**回信孔挂在谁身上**；而这决定"对端不在
    /// 了"能不能被看见（见 [`handshake::grant`] 上面那段）。`open` 是我自建（孔的
    /// 生命与我绑定，对端先死我看不见）；`dial` 是对端开（它退场时内核的寿命边封印
    /// 那扇门，我等在上面**当场醒**）。
    ///
    /// # 契约：首帧就是**那条"开一条会话"的请求**，它的答复随握手一起回来
    ///
    /// 本函数做完整件事：把首帧推出去（地址槽**留零** —— 协议自己的 `encode` 决定这个槽
    /// 在哪儿、什么时候写）→ 在对端入口孔上认领那枚回信孔 → 在**那枚孔上**收首帧的答复。
    ///
    /// **答复为什么在这条路上**：服务端收到首帧即建会话——它开回信孔、把句柄经入口孔交回
    /// （[`handshake::grant`]），并把"开成了"那句回执推进**它刚开的这枚孔**（`console` 的
    /// `State::open`）。两者是同一件事的两半，故本函数一次收齐。
    ///
    /// **调用方不得再发一遍首帧**——那正是这条契约曾经踩过的坑：首帧既是握手帧又是开门请求，
    /// 再来一遍就是**开第二条会话**；而第二遍的回执会被服务端推给一个按己方表解释的号，
    /// 客户端永远收不到，它收到的其实是第一遍的回执（表面一切正常，会话表却每开一次漏一格，
    /// 实测 `MAX_CLIENTS=8` ⇒ 同一条实例上第五次开会话必被表满拒）。
    ///
    /// `nonce` = 本端的认领号（调用方生成，同一个值也写进首帧）。**它为什么必须存在**
    /// 见 [`handshake::borrow`]：认领号与私有孔是**两道**独立的保险——前者让"哪一枚是我的"
    /// 不靠时序，后者让"这条路上只有本端与对端"不靠运气。
    ///
    /// `within` = **整件事**的上界（认领那枚孔 + 收首帧的答复，两段各分剩余预算）。
    /// **必须有界**：对端若拒了这条会话（表满、报文不合），那枚号永远不会来。
    ///
    /// # Errors
    /// - `Denied` — 入口门闩不在表里 / 无开辟者 / 对端没把句柄交回来 / 答复来源不是对端
    pub fn dial<W: Duet>(
        entry: &HolePie,
        first: &W::Req,
        nonce: u64,
        within: usize,
    ) -> EnvResult<(Port, W::Rep)> {
        let peer = mail::reserve(PieToken::new(entry.token()))?.1;
        if peer.get() == 0 {
            return Err(denied());
        }
        // **本端私有的握手孔**：那一条回执只走它，**不走请求孔**。
        //
        // 为什么必须私有（实测踩出来的）：请求孔是**双向**的——本端往里推请求，而
        // 服务端的请求循环正在它上面 `pull`。回执若推回请求孔，等于推进服务自己的收件箱，
        // 谁先 `pull` 谁拿走。读数：`[o] from=10`（服务建了会话）紧接
        // `[g3] not yours from=8 client=<本端的认领号>`——**8 就是服务自己**，它把自己的
        // 回执吸了回去并当成一条坏请求拒掉；本端随后等满上界（`[d3] borrow err`）。
        // 那不是时序抖动，是这条路的结构：一枚单槽信箱、两个方向的消费者。
        let control = HolePie::unseal()?;
        let to = ship(&control, peer, Access::READ | Access::WRITE, Policy::NONE)?;
        // 孔经**报文里的地址槽**递出去（`Duet::encode` 决定它在哪一格）。
        let mut wire = W::wire();
        let sent = W::encode(first, to.seed(), wire.as_mut());
        if entry.push(wire.as_ref().get(..sent).ok_or_else(denied)?).is_err() {
            let _ = control.release();
            return Err(denied());
        }
        let reply = match crate::core::handshake::borrow(&control, nonce, within) {
            Ok(r) => r,
            Err(e) => {
                let _ = control.release();
                return Err(e);
            }
        };
        // 握手孔用完即弃：对端那份在推完回执后也放下，孔随之回收。
        let _ = control.release();
        let port = Port::adopt(peer, HolePie::from_token(entry.token()), reply);
        let rep = port.take::<W>(within)?;
        Ok((port, rep))
    }

    /// 现成的一条会话：对端坐标 + 入口门闩 + **借来的**回信孔。
    ///
    /// 给 [`Port::dial`] 用；也给"句柄早就在手里"的场合（如启动期配给）留一条不再
    /// 建孔的路。
    pub fn adopt(peer: TaskId, entry: HolePie, reply: HolePie) -> Port {
        Port {
            to: To {
                peer,
                seed: PieToken::new(reply.token()),
            },
            entry,
            reply,
        }
    }

    /// 关：只放下回信孔（**级联已含对端那枚副本**，不必再 `revoke`——那是旧
    /// `Channel` 的冗余动作，它一旦失败还会短路后续清理）。
    pub fn close(self) -> EnvResult<()> {
        self.reply.release()
    }
}
