//! console·client — 线对侧：`Console` 会话 + `Readline` 结果。
//!
//! 与旧 `programs/src/term` 的 `Terminal`/`Readline` **同形**，但内部改走协议：
//! 客户端不再碰 UART，只往请求孔推请求、从回信孔收回复。
//!
//! # 回信孔是**借来的**
//!
//! 那枚孔由**服务端**开、把 `READ | WRITE` 副本授给本端、句柄经入口孔交回（[`Console::open`]
//! 的三步）。理由不是省事：孔的生命挂在对端身上，对端退场时内核封印它，本端**当场
//! 拿到 `Dead`** ⇒ 「对端还没回」与「对端已经没了」才分得开。旧形状（本端自建、
//! 授一枚给服务端）里这两件事长得一模一样，服务被打死后本端永久挂住——实测
//! `kill console` 之后 shell 再也不返回。
//!
//! # 同步点在哪
//!
//! `write` **等到 Ok 才返回**——这条很要紧：`write(prompt)` 返回时提示符已落屏，
//! 之后的 `readline` 才能安全地等输入。若 `write` 是"推完就算"，提示符会憋在孔里，
//! 而用户已经在对空行打字。
//!
//! 这正是「一次往返」的成熟形（[`Session`]），不新造机制。
//!
//! # 生命周期契约（显式，不靠 Drop）
//!
//! `HolePie` 没有 `Drop`——它是**句柄值**，不是 RAII 守卫。故本会话的资源由
//! [`Console::close`] 显式放：会话登记（服务侧）随 `Close` 撤回，回复孔随
//! [`Session::close`] 放下，请求孔那枚随进程退出由内核回收。**不假装**有 `Drop`——那是这套机制既有的形状，本模块照它写。

use alloc::string::String;

use env::{EnvResult, PieToken};

use runtime::env::mail::HolePie;

use super::{REPLY_MS, open};
use super::wire::{CAP, LINE, Query, Reply, denied};
use crate::session::{Session, push_within};

/// 认领号：**单调**即可（不必不可预测——它防的是"哪一帧是谁的"这种时序错配，
/// 不是伪造）。起点掺本任务 id，避免同一次启动里两个域撞上同一个号。
fn next_nonce() -> u64 {
    use core::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    static SEED: AtomicU64 = AtomicU64::new(0);
    let seed = SEED.load(Ordering::Relaxed);
    let seed = if seed == 0 {
        let t = runtime::env::task::self_id().map(|t| t.get() as u64).unwrap_or(1);
        let s = t.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        let _ = SEED.compare_exchange(0, s, Ordering::Relaxed, Ordering::Relaxed);
        SEED.load(Ordering::Relaxed)
    } else {
        seed
    };
    seed.wrapping_add(NEXT.fetch_add(1, Ordering::Relaxed) + 1)
}

/// **一声不吭**的到点次数上界（答了"还在读"就清零，见 `readline` 里 ③）。
///
/// 单次有界只挡住"对端没了"，挡不住"对端活着但不应答"——那条路上无界重发就是活锁。
/// 用尽即返错，由调用方走它那条有界的重连（`shell` 的 `CONSOLE_RETRY`）。
const READLINE_ROUNDS: usize = 4;

/// 一次 `ReadLine` 等的上界（毫秒）：超了就**重发同一条**（幂等，见服务侧
/// `State::readline`）。它**不是**"用户必须在这一段内敲完"——用户一个字没敲也不影响，
/// 重发不重置服务侧那一行的缓冲。取 3 s 是量出来的折中：短到门那一步（15 s）里能试
/// 好几次，长到正常打字期间根本不会触发。
const READLINE_WAIT_MS: usize = 3000;

/// **开一条会话**每一段的上界（毫秒）：推首帧 / 认领那枚孔 / 收首帧的答复各以它为界。
/// 比一次往返短：这一段里对端只做"开一枚孔 + 授出 + 推一条回执"，没有设备 I/O、
/// 也不等用户。给上界是为了让"服务把这条会话拒了"（表满 / 报文不合 ⇒ 那枚号永远不会来）
/// 落成一次可重试的失败，而不是永久挂起。
const HANDSHAKE_TIMEOUT_MS: usize = 500;

/// 读一行的结果。与旧 `term::Readline` 同形（`Line`/`Eof`/`Interrupt`）。
#[derive(Debug)]
pub enum Readline {
    /// 一行文本（回车提交，不含 `\n`）。
    Line(String),
    /// Ctrl-D。
    Eof,
    /// Ctrl-C。
    Interrupt,
}

/// 控制台会话：**请求那一枚孔** + **回复那一侧**（[`Session`]）+ 会话 id。
///
/// `entry` 由**父域经启动期握手配给**（与目录入口同一套 `Pier`），之后一切走本会话。
///
/// **三样东西各指回一个操作**：`entry` 是请求的去路（有界推）、`session` 是回复那一侧
/// （收 / 判活 / 收场）、`client` 是本会话在服务侧登记的那个号。
///
/// **"这条会话还算不算数"不在这里判**：回信孔由服务端开，服务退场时内核的寿命边封印它
/// （`gate::doom`）⇒ 本端下一次往返/等读**当场拿到 `Dead`**——那就是答案本身，不需要比
/// 任何代理量。曾经拿"入口号变没变"当凭据，而目录的 `Connect` 每次都转授**一份新副本**
/// ⇒ 那个号会因为完全无关的原因变化，是拿错了信号。
pub struct Console {
    entry: HolePie,
    session: Session,
    /// 会话 id（0 = 未开成）。
    client: usize,
}

impl Console {
    /// 打开会话：**一次往返**。
    ///
    /// [`open::open`] 把首帧（`Open`，带认领号、地址槽装本端的私有握手孔）推给服务；
    /// 服务端收到它**即建会话**——开一枚回信孔、把 `READ | WRITE` 副本授给本端、句柄连同
    /// 认领号经那枚私有孔交回，并把"开成了"那句回执（会话 id）推进**它刚开的那枚孔**。
    /// 故本函数一次收齐两样：回信孔（认领）与会话 id（首帧的答复）。
    ///
    /// # 回信孔为什么由服务端开
    ///
    /// 它服务于"服务端出话、客户端收话"，故开者必须是服务端：服务退场时内核的寿命边
    /// 封印它（"开者退场 ⇒ 它开的资源一起封印"，见 `gate::doom`），睡在它上面的客户端
    /// **当场拿到 `Dead`** ⇒ 会话被判为断了、走本类型外面那条重连。反过来（本端自建）时
    /// 服务被打死这扇门不死，本端永久挂在 `Busy` 上，「对端还没回」与「对端已经没了」
    /// 不可区分。
    ///
    /// 这一段（认领号 + 私有握手孔 + 回信孔的交接）是**本协议自己的**，见 [`super::open`]。
    pub fn open(entry: HolePie) -> EnvResult<Console> {
        // 认领号：当场生成（本进程每开一次会话一个），同一个值同时进首帧与认领那一趟
        // ——服务端会把它连同回信孔句柄一起推回来。
        let nonce = next_nonce();
        let (session, rep) = open::open(&entry, nonce, HANDSHAKE_TIMEOUT_MS)?;
        match rep {
            Reply::Ok { client } => Ok(Console { entry, session, client }),
            _ => Err(denied()),
        }
    }

    /// 本会话 id（0 = 未开成）。
    pub fn client(&self) -> usize {
        self.client
    }

    /// 本协议的一次往返：编帧 → **有界**推 → **有界**收（[`Session::pull`] 内建核来源与三态）
    /// → 解码。`within` 是每一半的上界：写一次用 [`REPLY_MS`]，等一行用 [`READLINE_WAIT_MS`]。
    fn call(&self, query: &Query, within: usize) -> EnvResult<Reply> {
        // 回信地址只在 `Open` 上带（那一帧的地址槽装握手孔），其余动词那一格归会话号
        // ——`Query::encode` 的 `at` 对它们不落笔，故这里给 0。
        let (frame, n) = query.encode(PieToken::new(0));
        push_within(&self.entry, frame.get(..n).ok_or_else(denied)?, within)?;
        let mut buf = [0u8; CAP];
        Reply::decode(self.session.pull(&mut buf, within)?).map_err(|_| denied())
    }

    /// 写字符串：**阻塞到服务确认落屏**。
    ///
    /// 按 [`LINE`] 分片；每片一次往返。旧 `io::put` 是同步写设备，
    /// 故这里的同步语义与它同级——`write(prompt)` 返回即可安全开始读。
    pub fn write(&self, s: &str) -> EnvResult<()> {
        if self.client == 0 {
            return Err(denied());
        }
        for chunk in s.as_bytes().chunks(LINE) {
            let query = Query::write(self.client, chunk).ok_or_else(denied)?;
            match self.call(&query, REPLY_MS)? {
                Reply::Ok { .. } => {}
                _ => return Err(denied()),
            }
        }
        Ok(())
    }

    /// 读一行（带行编辑）。**每一次等都有界、重发有总次数**（见函数内三段理由）。
    ///
    /// `prompt` 随请求带上（服务侧重绘要用它）：本行先把它同步写到设备上，
    /// 再请求读行——两件事的次序不能颠倒（否则用户对着空行打字）。
    pub fn readline(&self, prompt: &str) -> EnvResult<Readline> {
        if self.client == 0 {
            return Err(denied());
        }
        if !prompt.is_empty() {
            self.write(prompt)?;
        }
        let bytes = prompt.as_bytes();
        let query =
            Query::readline(self.client, &bytes[..bytes.len().min(LINE)]).ok_or_else(denied)?;
        // **有界等 + 重发同一条 + 总次数只数"一声不吭"**（不是轮询：重发是幂等的，服务侧
        // 见 `State::readline`）。三段各有各的理由：
        //
        // ① 单次有界：`usize::MAX` 把"用户还没敲完"与"服务已经没了"折成同一件事。有界之后
        //    `Busy` 的含义收窄成"**探过、它还在**，只是没回"，而"没了"当场拿到 `Dead`/
        //    `Denied` ⇒ 调用方走重连。
        // ② 重发同一条：服务侧对"同会话重问同一条"是幂等的（已有等读会话 ⇒ 不排队、不重置
        //    那一行的缓冲），故重发既不丢用户已敲的字，也不占第二格。它现在还**答一句
        //    "还在读"**（`Reply::Waiting`）——见 ③。
        // ③ 只把**一声不吭**的到点计入总界：服务答了"还在读"就清零。不这样分，`Busy` 会把
        //    "用户发呆"与"服务不应答"折成同一件事——前者每 4 × 3 s 就被误判成会话断了、
        //    重连一次（`shell` 打一行 reconnected，两侧还各多留一两枚探针孔）。真正卡死的
        //    服务一句都不答，照样用尽总界；用尽即返错——终局是停机，不是活锁。
        let mut silent = 0usize;
        loop {
            match self.call(&query, READLINE_WAIT_MS) {
                Ok(Reply::Line { text }) => {
                    let text = core::str::from_utf8(text.as_bytes()).map_err(|_| denied())?;
                    return Ok(Readline::Line(String::from(text)));
                }
                Ok(Reply::Eof) => return Ok(Readline::Eof),
                Ok(Reply::Interrupt) => return Ok(Readline::Interrupt),
                // 服务答了"还在读"：它在等我 —— 这一句就是判据，把计数清零。
                Ok(Reply::Waiting) => silent = 0,
                Ok(_) => return Err(denied()),
                Err(e) if e.source.is_busy() => {
                    silent += 1;
                    if silent >= READLINE_ROUNDS {
                        return Err(e);
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// 关会话：撤回服务侧的会话登记。
    ///
    /// 回复孔随 [`Session::close`] 放下，请求孔那枚随进程退出由内核回收（`HolePie` 不是
    /// RAII 守卫，见文件头）。
    pub fn close(&self) -> EnvResult<()> {
        if self.client == 0 {
            return Ok(());
        }
        match self.call(
            &Query::Close {
                client: self.client,
            },
            REPLY_MS,
        )? {
            Reply::Ok { .. } => {
                let _ = self.session.close();
                Ok(())
            }
            _ => Err(denied()),
        }
    }
}
