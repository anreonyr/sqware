//! handshake — 启动期握手：父域**开上行孔**、子域**靠泊**、两条报文。
//!
//! "引荐"那一对报文（`Refer` / `Referred`）住 `protocol::dispatch::control`：它们是
//! 目录协议的语义，不是机制。
//!
//! 目标（B 版）：**父域 root 手里零服务孔**。子域自建**控制孔**并把父侧句柄交给
//! root；客户端要用的目录门闩由 **dir 亲授**（root 只转达「授给谁」）——客户端拿到
//! 的副本 `vestor == dir`，来源可自证。
//!
//! 线格式：`[0] = tag`、`[1..9] = payload u64 LE`——**定长 9 字节**，两条报文同一形状。
//!
//! ```text
//! Quay      子 → 父   子域上行孔      我控制孔在父侧的句柄
//! Pier      父 → 子   子域下行孔      目录门闩在本侧的句柄（0 = 无）
//! Grant     服务 → 客户端  **握手孔**  我给你开的回信孔，在你表里的句柄
//! ```
//!
//! **`Grant` 走哪一枚孔是量出来的**：它一度推在**请求孔**上，而请求孔是一枚单槽信箱、
//! 服务端的请求循环正在上面 `pull` ⇒ 回执常被**服务自己**吸回去当成一条坏请求拒掉，
//! 客户端等满上界（实测：同一次会话照 50% 的概率开不成）。现在它推给客户端为这次握手
//! **私有**的那枚孔——枚号经 `Open` 帧的地址槽递过来（见 `port::Port::dial`）。
//!
//! **每条孔单一发送者**：上行孔只有子域推、下行孔只有父域推、dir 控制孔只有 root 推。
//! 「谁能推谁就是谁」是结构性的，故报文里不带任何身份。
//!
//! **`Grant` 与另两条不是一类**：它不搬会话，它搬**一扇门的开者身份**（见
//! [`grant`]/[`borrow`]）。放进本模块是因为线形与收发完全同款；而"谁开回信孔"
//! 这条语义归协议（说出来的那一侧），本模块只给手法。
//!
//! 为什么上行孔与下行孔必须分开：Hole 是单槽信箱，同一条孔上既 push 又 pull 会把
//! 自己刚写的消息读回来（T2 实测死锁）。这也正是既有 IPC 的形态：请求孔与回信孔
//! 是两条不同的孔。
//!
//! 通道用完不回收：门闩由各任务的权限表保活到关机，启动通道没有后续语义。

use env::{EnvError, EnvResult, PieToken, TaskId, make_err};

use crate::core::port::{Access, Policy, ship};
use crate::env::mail::{self, HolePie};

/// 报文长度：1 字节 tag + 一个 u64（本模块自用：两条报文的线形都定长）。
///
/// **它就是长度**：本模块的报文没有变长那一段，故这里一个数既是容量也是被推的字节数
/// （与各协议不同——那边 `CAP` 只是容量上界，长度由成帧的人报）。
const LEN: usize = 9;

const TAG_QUAY: u8 = 1;
const TAG_PIER: u8 = 2;
const TAG_GRANT: u8 = 3;

/// `Grant` 那一形状的长度：`tag` + **认领号** + `u64`。
///
/// 比另两条多一个 `u64`，那是**认领号**（nonce）：客户端每次开口给一个当场生成的号，
/// 服务端原样回。理由见 [`borrow`]——入口孔上会有别的帧（实测：一条裸 `Close` 正好
/// 也是 9 字节、`b0=3`，把"下一帧就是答复"骗过去了）。
const GRANT_LEN: usize = 1 + 8 + 8;

/// 枚举自己权限表的上限（防越界扫描跑飞；表量级个位数）。
const MAX_PIES: usize = 64;

const E_DENIED: isize = -1;

fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(E_DENIED))
}

/// 发一条报文：`[tag][payload]`。
///
/// `payload` 收任何句柄：线形是 8 字节，但**类型不化**——调用方继续拿 `PieToken`。
fn send(hole: &HolePie, tag: u8, payload: impl Into<usize>) -> EnvResult<()> {
    let mut buf = [0u8; LEN];
    buf[0] = tag;
    buf[1..].copy_from_slice(&payload.into().to_le_bytes());
    hole.push(&buf)
}

/// 收一条报文：读满 `LEN` 并校验 tag，返 payload。tag 不符 / 短读 → `Denied`。
fn recv_pie(hole: &HolePie, tag: u8) -> EnvResult<PieToken> {
    recv(hole, tag).map(PieToken::new)
}

fn recv(hole: &HolePie, tag: u8) -> EnvResult<usize> {
    let mut buf = [0u8; LEN];
    // 长度由内核给（谁推的谁知道多长），本模块只认"[tag][u64]"这一种形状。
    if hole.pull(&mut buf)? != LEN || buf[0] != tag {
        return Err(denied());
    }
    Ok(usize::from_le_bytes(
        buf[1..].try_into().unwrap_or([0u8; 8]),
    ))
}

/// 子 → 父：报到。`hole` = 我自建控制孔**在父侧**的句柄（`Accord` 的返回值）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Quay {
    hole: PieToken,
}

impl Quay {
    pub fn new(hole: impl Into<usize>) -> Self {
        Self {
            hole: PieToken::new(hole.into()),
        }
    }

    pub fn hole(&self) -> PieToken {
        self.hole
    }

    /// 子侧：在子域上行孔上报到。
    pub fn push(self, up: &HolePie) -> EnvResult<()> {
        send(up, TAG_QUAY, self.hole)
    }

    /// 父侧：在子域上行孔上收报到。
    pub fn pull(up: &HolePie) -> EnvResult<Quay> {
        Ok(Quay {
            hole: recv_pie(up, TAG_QUAY)?,
        })
    }
}

/// 父 → 子：配给。`token` = 目录门闩**在子侧**的句柄（0 = 无）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Pier {
    token: PieToken,
}

impl Pier {
    pub fn new(token: impl Into<usize>) -> Self {
        Self {
            token: PieToken::new(token.into()),
        }
    }

    pub fn token(&self) -> PieToken {
        self.token
    }

    /// 父侧：往子域下行孔里配给。
    pub fn push(self, down: &HolePie) -> EnvResult<()> {
        send(down, TAG_PIER, self.token)
    }

    /// 子侧：从下行孔里收配给。
    pub fn pull(down: &HolePie) -> EnvResult<Pier> {
        Ok(Pier {
            token: recv_pie(down, TAG_PIER)?,
        })
    }
}

/// 父侧：为子域开上行孔并把 `R|W|VEST` 副本授给它。
///
/// **带 `VEST`**：子域要把这条孔再授给自己的**控制线程**（同域跨 task 门闩不共享），
/// 而 `Accord` 的门槛是"源门闩持 `VEST`"。
///
/// **时序义务**：必须早于 `Hatch(child)`——否则子域起跑时 `moor()` 找不到它。
pub fn dock(child: TaskId) -> EnvResult<HolePie> {
    let up = HolePie::unseal()?;
    ship(&up, child, Access::READ | Access::WRITE, Policy::VEST)?;
    Ok(up)
}

/// 子侧：认父域给的上行孔——本任务表里 `vestor == sire()` **且** `owner == sire()`
/// 的那一枚。
///
/// 两个条件缺一不可：`vestor` 说「这枚门闩是父域授给我的」，`owner` 说「这扇门是
/// 父域自己开的」。父域**转授**来的门闩（如目录请求门闩）`vestor` 也是父域，但
/// 它的 `owner` 是别人——只看 `vestor` 会认错。
///
/// 顶级域（`sire() == 0`）没有上行孔 → `Denied`。
pub fn moor() -> EnvResult<HolePie> {
    let sire = crate::env::task::sire()?.get();
    if sire == 0 {
        return Err(denied());
    }
    for index in 0..MAX_PIES {
        let (token, _, vestor) = mail::collect(index)?;
        if token.get() == 0 {
            break;
        }
        if vestor.get() != sire {
            continue;
        }
        if let Ok((_, owner)) = mail::reserve(token)
            && owner.get() == sire
        {
            return Ok(HolePie::from_token(token.get()));
        }
    }
    Err(denied())
}

// ── 回信孔的归属：谁开谁负责 ─────────────────────────────────────────────
//
// 回信孔服务于"**对端出话、本端收话**"，于是它挂在谁身上，就决定了"对端不在了"
// 这件事能不能被收话的那一端看见。
//
// 旧形状是**客户端开**（`port::Port::open` 自建、`ship` 一枚给服务端）：孔的生命与
// 客户端绑定，服务端被打死它**不死**——客户端 `pull` 睡在一扇再不会有推者的门上，
// 拿到的是永久的 `Busy` 而不是 `Dead`。「对端还没回」与「对端已经没了」因此不可
// 区分，表现是客户端永久挂住（实测：`kill console` 之后 shell 再不返回，
// `shell: console reconnected` 一次都没出现）。
//
// 反过来之后服务端是开者，而开者退场由内核的寿命边收掉（`gate::doom` ——"开者退场
// ⇒ 它开的资源一起封印"）⇒ 等在它上面的客户端**当场醒**，拿到 `Dead`。
//
// 代价是客户端得先要到这枚句柄。两个数够把这件事说清：**服务端开的孔**与**客户端表
// 里的号**；而后者只能由客户端认（`from_token` 是零成本重建，故两边对同一个号的
// 理解必须一致——它就是同一个号）。

/// 服务侧：为**这条会话的对端**开一枚回信孔，把 `READ | WRITE` 副本授给它，并把这枚
/// 句柄（连同 `nonce`）经 `control` 交回去。返**我这侧**那枚。
///
/// # 为什么是 `READ | WRITE`
///
/// 翻面之后授出去的这一份**就是客户端唯一的句柄**——它要 `pull` 才收得到回复，而
/// `Pull` 的方向位是 R（`envcall::mail::wait_dir`：`HoleDir::Pull => Need::Read`）。
/// 只授 W 的表现是**当场 `Denied`**（连等都不等，不是 `Busy`）：实测
/// `[r9] … PULL ERR code=-1`。旧形状里"只授 W"不算错——那时客户端自己开着这枚孔、
/// 手里那份是满权限，授出去那份只被服务端用来推。写的是同一行字，读法随"谁开孔"翻面。
///
/// `control` = 对端**为这次握手自己开的那枚私有孔**（枚号随 `Open` 帧的地址槽递过来，
/// 见 `port::Port::dial`）——回执只走它。
///
/// **它一度是"对端请求进来的那条孔"**（本服务自己开的那扇门），而那条路是错的：请求孔是
/// 一枚单槽信箱、本服务的请求循环正在上面 `pull`，回执推进去常被**自己**吸回来当成一条
/// 坏请求拒掉，客户端等满上界（实测：服务建完会话紧接着 `[g3] not yours from=<服务自己>`
/// 一条，同一次会话照 50% 的概率开不成）。私有孔把这件事变成一问一答。
///
/// # Errors
/// - `Denied` — `ship` 授不出去（`peer` 不是个活任务）或 `control` 推不动（对端已走）
pub fn grant(control: &HolePie, peer: env::TaskId, nonce: u64) -> EnvResult<HolePie> {
    let reply = HolePie::unseal()?;
    let to = ship(&reply, peer, Access::READ | Access::WRITE, Policy::NONE)?;
    let mut buf = [0u8; GRANT_LEN];
    buf[0] = TAG_GRANT;
    buf[1..9].copy_from_slice(&nonce.to_le_bytes());
    buf[9..].copy_from_slice(&to.seed().get().to_le_bytes());
    control.push(&buf)?;
    Ok(reply)
}

/// 客户端：在**自己那枚握手孔**上收服务端交回的回信孔句柄，**并认领号**。
///
/// # 为什么要有认领号（实测踩出来的）
///
/// 这条孔上还会有别的帧吗？**现在不会了**（它是本端为这次握手私有的，只有服务端往里推一
/// 条），而当初会：它一度就是请求孔——本端往里推请求、服务端把答复推回来，"下一帧就是我
/// 的答复"因此是个**假前提**，实测一条**裸 `Close`**（`n=9`、`b0=3`）落在同一个槽里，
/// `recv` 一看形状不符就答 `Denied`，**整条会话当场开不成**（`Console::open` 返回 `-1`，
/// shell 启动即退场 ⇒ 级联停机），同一份产物五次启动里中招两三次。
///
/// 认领号与私有孔是**两道**保险，都留着：前者让"哪一枚是我的"不靠时序，后者让"这条路上
/// 只有我和对端"不靠运气。
///
/// 故本函数**不信任下一帧**：它逐帧读到一枚 tag 与 `nonce` 都对得上的才认领，别的一律
/// 丢掉。`nonce` 由客户端在开口那一帧里给出（`Query::Open` 带它），服务端原样回——
/// 对端不可能猜到，故"谁在什么时候等哪一枚"这件事不再靠时序。
///
/// # 有界
///
/// 服务端若拒了这条会话（表满 / 报文不合），这枚号永远不会来。无界等就把"对面没开成"
/// 变成一次永久挂起——故 `within` 是调用方的义务（见 `console::client::open`）。
///
/// # Errors
/// - `Denied` — 界内没等到、握手孔已死
pub fn borrow(control: &HolePie, nonce: u64, within: usize) -> EnvResult<HolePie> {
    let deadline = crate::env::chrono::clock()?
        .0
        .saturating_add((within as u64).div_ceil(1000));
    loop {
        let now = crate::env::chrono::clock()?.0;
        if now > deadline {
            return Err(denied());
        }
        let remain_s = (deadline - now).max(1);
        let mut buf = [0u8; GRANT_LEN];
        // 逐帧读：**短帧也照读**（`LEN` 那一形状的帧只有 9 字节），不是 `Grant` 就丢。
        let Ok(n) = control.pull_timeout(&mut buf, (remain_s * 1000) as usize) else {
            return Err(denied());
        };
        let Some(msg) = buf.get(..n) else { continue };
        if n != GRANT_LEN || msg[0] != TAG_GRANT {
            continue;
        }
        if u64::from_le_bytes(msg[1..9].try_into().unwrap_or([0u8; 8])) != nonce {
            continue;
        }
        let token = usize::from_le_bytes(msg[9..].try_into().unwrap_or([0u8; 8]));
        return Ok(HolePie::from_token(token));
    }
}
