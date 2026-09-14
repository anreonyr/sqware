//! console·client — 线对侧：`Console` 会话 + `Readline` 结果。
//!
//! 与旧 `programs/src/term` 的 `Terminal`/`Readline` **同形**，但内部改走协议：
//! 客户端不再碰 UART，只往请求孔推请求、从回信孔收回复。
//!
//! # 回信孔是**借来的**
//!
//! 那枚孔由**服务端**开、把 `WRITE` 副本授给本端、句柄经入口孔交回（[`Console::open`]
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
//! 这正是「一次往返」的成熟形（[`Port`]），不新造机制。
//!
//! # 生命周期契约（显式，不靠 Drop）
//!
//! `HolePie` 没有 `Drop`——它是**句柄值**，不是 RAII 守卫。故本会话的资源由
//! [`Console::close`] 显式放：会话登记（服务侧）随 `Close` 撤回，两枚孔随进程退出
//! 由内核回收。**不假装**有 `Drop`——那是这套机制既有的形状，本模块照它写。

use alloc::string::String;

use env::EnvResult;

use runtime::core::port::Port;
use runtime::env::mail::HolePie;

use super::wire::{LINE, Query, Reply, denied};

/// 一次往返的上界（毫秒）。与 dispatch 的 `REPLY_TIMEOUT_MS` 同值。
const REPLY_TIMEOUT_MS: usize = 1000;

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

/// 一次 `ReadLine` 等的上界（毫秒）：超了就**重发同一条**（幂等，见服务侧
/// `State::readline`）。它**不是**"用户必须在这一段内敲完"——用户一个字没敲也不影响，
/// 重发不重置服务侧那一行的缓冲。取 3 s 是量出来的折中：短到门那一步（15 s）里能试
/// 好几次，长到正常打字期间根本不会触发。
const READLINE_WAIT_MS: usize = 3000;

/// 等**回信孔句柄**的上界（毫秒）。比一次往返短：这一段里对端只做"开一枚孔 +
/// 授出 + 推 9 字节"，没有设备 I/O、也不等用户。给上界是为了让"服务把这条会话
/// 拒了"（表满 / 报文不合 ⇒ 那枚号永远不会来）落成一次可重试的失败，而不是永久挂起。
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

/// 控制台会话：一次往返的机制（[`Port`]）+ 会话 id。
///
/// `entry` 由**父域经启动期握手配给**（与目录入口同一套 `Pier`），之后一切走本会话。
pub struct Console {
    port: Port,
    /// 会话 id（0 = 未开成）。
    client: usize,
}

impl Console {
    /// 打开会话。
    ///
    /// 三步，次序是契约（违反它的表现是握手等到超时，不是死锁）：
    ///   1. [`Port::dial`] 把**首帧**（`Open`，地址槽留零）推给服务，并在入口孔上
    ///      等一枚句柄回来 —— 那枚就是**服务端为我开的**回信孔（见下）；
    ///   2. 服务端已经为这条会话开好回信孔、把 `WRITE` 副本授给我、并把句柄交回；
    ///   3. 首次 [`Port::call`] 发那条 `Open` 的数据帧，拿到会话 id。
    ///
    /// # 回信孔为什么由服务端开
    ///
    /// 它服务于"服务端出话、客户端收话"，故开者必须是服务端：服务退场时内核的寿命边
    /// 封印它（"开者退场 ⇒ 它开的资源一起封印"，见 `gate::doom`），睡在它上面的客户端
    /// **当场拿到 `Dead`** ⇒ 会话被判为断了、走 [`crate::console::client::Console`] 外面
    /// 那条重连。反过来（本端自建）时服务被打死这扇门不死，本端永久挂在 `Busy` 上，
    /// 「对端还没回」与「对端已经没了」不可区分。
    ///
    /// 对端 task id 由 [`Port::dial`] 用 `Reserve(entry).owner` 求得——**门闩的开辟者
    /// 就是服务本身**（root 转发不改 `owner`，只改 `vestor`）。
    pub fn open(entry: HolePie) -> EnvResult<Console> {
        // 认领号：当场生成（本进程每开一次会话一个），同一个值同时进首帧与 `dial`
        // ——服务端会把它连同回信孔句柄一起推回来，见 [`Port::dial`]。
        let nonce = next_nonce();
        let port = Port::dial::<Query>(&entry, &Query::open(nonce), nonce, HANDSHAKE_TIMEOUT_MS)?;
        let mut console = Console { port, client: 0 };
        // 首帧已经推过了，这一条**不再**带认领号：握手那一步已完成。
        match console.call(&Query::open(0))? {
            Reply::Ok { client } => {
                console.client = client;
                Ok(console)
            }
            _ => Err(denied()),
        }
    }

    /// 本会话 id（0 = 未开成）。
    pub fn client(&self) -> usize {
        self.client
    }

    /// 一次往返：请求推请求孔、回复从自己的回信孔**有界**收、**核来源**。
    fn call(&self, query: &Query) -> EnvResult<Reply> {
        self.port.call::<Query>(query, REPLY_TIMEOUT_MS)
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
            match self.call(&query)? {
                Reply::Ok { .. } => {}
                _ => return Err(denied()),
            }
        }
        Ok(())
    }

    /// 读一行（带行编辑）。**无上界**——用户在打字，服务端阻塞等回车。
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
        // **有界等 + 重发同一条**（不是轮询：重发是幂等的，服务侧见 `State::readline`）。
        //
        // 为什么要上界：`usize::MAX` 把"用户还没敲完"与"服务已经没了"折成同一件事
        // ——后者本该由内核封印回信孔来唤醒（`gate::doom` 的寿命边），但那条路一旦
        // 没走到（竞态），永久等就是**永久挂住**：实测门里那一步卡死 90 s，读数里
        // 一次 `readline broke` 都没有。
        //
        // 有界之后 `Busy` 的含义回到"这一段没有回复"：**重发同一条**即可——服务侧
        // 对"同会话重问同一条"是幂等的（已有等读会话 ⇒ 不排队、不重置那一行的缓冲），
        // 故重发既不丢用户已敲的字，也不占第二格。只要服务真死了，重发会拿到
        // `Dead`/`Denied`，本函数当场返回 `Err`，调用方走 `reconnect()`。
        loop {
            match self.port.call::<Query>(&query, READLINE_WAIT_MS) {
                Ok(Reply::Line { text }) => {
                    let text = core::str::from_utf8(text.as_bytes()).map_err(|_| denied())?;
                    return Ok(Readline::Line(String::from(text)));
                }
                Ok(Reply::Eof) => return Ok(Readline::Eof),
                Ok(Reply::Interrupt) => return Ok(Readline::Interrupt),
                Ok(_) => return Err(denied()),
                // 超时：服务可能还活着（用户就是没敲完）⇒ 重问同一条。
                Err(e) if e.source.is_busy() => continue,
                Err(e) => return Err(e),
            }
        }
    }

    /// 关会话：撤回服务侧的会话登记。
    ///
    /// 两枚孔的副本随进程退出由内核回收（`HolePie` 不是 RAII 守卫，见文件头）。
    pub fn close(&self) -> EnvResult<()> {
        if self.client == 0 {
            return Ok(());
        }
        match self.call(&Query::Close {
            client: self.client,
        })? {
            Reply::Ok { .. } => Ok(()),
            _ => Err(denied()),
        }
    }
}
