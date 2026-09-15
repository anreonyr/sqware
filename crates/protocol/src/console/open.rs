//! console·open — 开会话：握手的**两端**。
//!
//! 名字是这一格的判据：**"开会话"是 console 自己的形状**（私有握手孔 + 认领号 + 首帧兼
//! 开门请求），而"会话"这个词归 [`crate::session`]（五家共用的那三格：判活 + 收场）。
//! 故本模块原名（`console::session`）让给了那一件。
//!
//! # 为什么这一段住在协议里
//!
//! 一段"开了要长期用"的会话怎么建立起来，是**控制台这一个协议**的语义：它要一枚
//! **私有的握手孔**（回执不从请求孔回来）、一个**认领号**（"哪一枚是我的"不靠时序）、
//! 还要在一帧里同时问出"开成没有"与"回信孔在哪"。别的协议是一问一答，用不上这一套。
//!
//! # 回信孔为什么由服务端开
//!
//! 它服务于"服务端出话、客户端收话"，故开者必须是服务端：服务退场时内核的寿命边封印
//! 它（"开者退场 ⇒ 它开的资源一起封印"，`gate::doom`），睡在它上面的客户端**当场拿到
//! `Dead`** ⇒ 本端把会话判为断了、走 `client` 那条重连。反过来（客户端自建）时服务被
//! 打死这扇门**不死**，客户端永久挂在 `Busy` 上——「对端还没回」与「对端已经没了」
//! 不可区分（实测：`kill console` 之后 shell 再也不返回）。
//!
//! 代价是客户端得先要到这枚句柄。下面两条报文就是为此存在的——它们在**入口孔与本端
//! 私有孔**之间往返一趟：
//!
//! ```text
//! 客户端                          服务端
//!   │ 首帧 Open{nonce}（地址槽 = 我的私有孔）→ 入口孔
//!   │                            建会话：开回信孔、授 R|W 给客户端
//!   │ ← Grant[tag][nonce][回信孔在对端表里的号]      推回**私有孔**
//!   │ ← Ok{client}                                     推回**新开的回信孔**
//! ```
//!
//! 首帧**不得再发第二遍**：它就是那条"开一条会话"的请求。发第二遍 = 开第二条会话，
//! 而第二遍的回执会被推给一个按己方表解释的号（永远收不到）；本端拿到的其实是第一遍的
//! 回执——表面一切正常，服务侧的会话表却每开一次漏一格（实测 `MAX_CLIENTS = 8` ⇒
//! 同一条实例上第五次开必被表满拒）。

use env::{EnvError, EnvResult, PieToken, TaskId, make_err};
use runtime::core::port::{Access, Policy, ship};
use runtime::env::mail::{self, AnyPie as _, HolePie};

use super::wire::{CAP, Query, Reply};
use crate::session::{Session, push_within};

/// 本协议的负码：协议错（与内核码同表）。
fn denied() -> erra::Error<EnvError> {
    make_err(EnvError::from_raw(-1))
}

/// 握手报文的形状（定长）：`[tag][认领号 u64][回信孔在对端表里的号 u64]`。
///
/// **长度就是这一格**：本形状没有变长那一段，故一个数既是容量也是被推的字节数。
const TAG_GRANT: u8 = 3;
const GRANT_LEN: usize = 1 + 8 + 8;

/// 服务侧：为**这条会话的对端**开一枚回信孔，把 `READ | WRITE` 副本授给它，并把这枚
/// 句柄（连同认领号）经 `control` 交回去。返**本服务这侧**那枚。
///
/// # 为什么是 `READ | WRITE`
///
/// 翻面之后授出去的这一份**就是客户端唯一的句柄**——它要 `pull` 才收得到回复，而
/// `Pull` 的方向位是 R（`envcall::mail::wait_dir`：`HoleDir::Pull => Need::Read`）。
/// 只授 W 的表现是**当场 `Denied`**（连等都不等，不是 `Busy`）：实测
/// `[r9] … PULL ERR code=-1`。旧形状里"只授 W"不算错——那时客户端自己开着这枚孔、
/// 手里那份是满权限，授出去那份只被服务端用来推。写的是同一行字，读法随"谁开孔"翻面。
///
/// `control` = 对端在握手帧**地址槽**里递过来的那枚孔（本服务表里的号）——回执只走它。
/// **它一度是"对端请求进来的那条孔"**（本服务自己开的那扇门），而那条路是错的：请求孔是
/// 一枚单槽信箱、本服务的请求循环正在上面 `pull`，回执推进去常被**自己**吸回来当成一条
/// 坏请求拒掉，客户端等满上界（实测：服务建完会话紧接着 `[g3] not yours from=<服务自己>`
/// 一条，同一次会话照 50% 的概率开不成）。私有孔把这件事变成一问一答。
///
/// # Errors
/// - `Denied` — `ship` 授不出去（`peer` 不是个活任务）或 `control` 推不动（对端已走）
pub fn grant(control: &HolePie, peer: TaskId, nonce: u64) -> EnvResult<HolePie> {
    let reply = HolePie::unseal()?;
    let to = ship(&reply, peer, Access::READ | Access::WRITE, Policy::NONE)?;
    let mut buf = [0u8; GRANT_LEN];
    buf[0] = TAG_GRANT;
    buf[1..9].copy_from_slice(&nonce.to_le_bytes());
    buf[9..].copy_from_slice(&to.seed().get().to_le_bytes());
    control.push(&buf)?;
    Ok(reply)
}

/// 客户端：开一条会话。做完整件事：**首帧推出去** → 认领服务端交回的回信孔 →
/// 在**那枚孔上**收下首帧的答复。
///
/// `nonce` = 本端的认领号（调用方当场生成，同一个值也写进首帧）。
///
/// `within` = **每一段**的上界（推首帧 / 认领那枚孔 / 收首帧的答复，各以它为界）。
/// **必须有界**：对端若拒了这条会话（表满、报文不合），那枚号永远不会来。
///
/// # Errors
/// - `Denied` — 入口门闩不在表里 / 无开辟者 / 对端没把句柄交回来 / 答复来源不是对端
pub fn open(entry: &HolePie, nonce: u64, within: usize) -> EnvResult<(Session, Reply)> {
    let peer = mail::reserve(PieToken::new(entry.token()))?.1;
    if peer.get() == 0 {
        return Err(denied());
    }
    // **本端私有的握手孔**：那一条回执只走它，**不走请求孔**。
    //
    // 为什么必须私有（实测踩出来的）：请求孔是**双向**的——本端往里推请求，而服务端的
    // 请求循环正在它上面 `pull`。回执若推回请求孔，等于推进服务自己的收件箱，谁先
    // `pull` 谁拿走。读数：`[o] from=10`（服务建了会话）紧接
    // `[g3] not yours from=8 client=<本端的认领号>`——**8 就是服务自己**，它把自己的
    // 回执吸了回去并当成一条坏请求拒掉；本端随后等满上界（`[d3] borrow err`）。
    // 那不是时序抖动，是这条路的结构：一枚单槽信箱、两个方向的消费者。
    let control = HolePie::unseal()?;
    let to = ship(&control, peer, Access::READ | Access::WRITE, Policy::NONE)?;
    // 孔经**首帧的地址槽**递出去（只在 `Open` 上带，见 `wire::put_address`）。
    let (frame, n) = Query::open(nonce).encode(to.seed());
    if push_within(entry, frame.get(..n).ok_or_else(denied)?, within).is_err() {
        let _ = control.release();
        return Err(denied());
    }
    let reply = match pull_grant(&control, nonce, within) {
        Ok(r) => r,
        Err(e) => {
            let _ = control.release();
            return Err(e);
        }
    };
    // 握手孔**不放下**：对端表里已经留了一枚副本，而那枚正是服务侧判活的探针
    // （`Session::probe` 只认"对端开的"那一枚）。放下本端这一份会把对端那枚一起级联摘掉。
    let session = Session::new(peer, PieToken::new(reply.token()), PieToken::new(reply.token()));
    let mut buf = [0u8; CAP];
    let rep = Reply::decode(session.pull(&mut buf, within)?).map_err(|_| denied())?;
    Ok((session, rep))
}

/// 客户端：在**自己那枚握手孔**上收服务端交回的回信孔句柄，**并认领号**。
///
/// # 为什么要有认领号（实测踩出来的）
///
/// 这条孔上还会有别的帧吗？**现在不会了**（它是本端为这次握手私有的，只有服务端往里推
/// 一条），而当初会：它一度就是请求孔——本端往里推请求、服务端把答复推回来，"下一帧就是
/// 我的答复"因此是个**假前提**，实测一条**裸 `Close`**（`n=9`、`b0=3`）落在同一个槽里，
/// 一看形状不符就答 `Denied`，**整条会话当场开不成**（`Console::open` 返回 `-1`，
/// shell 启动即退场 ⇒ 级联停机），同一份产物五次启动里中招两三次。
///
/// 认领号与私有孔是**两道**保险，都留着：前者让"哪一枚是我的"不靠时序，后者让"这条路上
/// 只有我和对端"不靠运气。故本函数**不信任下一帧**：逐帧读到一枚 tag 与 `nonce` 都对得上
/// 的才认领，别的一律丢掉。
fn pull_grant(control: &HolePie, nonce: u64, within: usize) -> EnvResult<HolePie> {
    let deadline = runtime::env::chrono::clock()?
        .0
        .saturating_add((within as u64).div_ceil(1000));
    loop {
        let now = runtime::env::chrono::clock()?.0;
        if now > deadline {
            return Err(denied());
        }
        let remain_s = (deadline - now).max(1);
        let mut buf = [0u8; GRANT_LEN];
        // 逐帧读：**短帧也照读**（别的形状的帧可能更短），不是 `Grant` 就丢。
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
