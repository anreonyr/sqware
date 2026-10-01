//! supply::server — **引导域那一侧**：照单取源、授出、回一张回单（常驻循环 [`serve`]）
//! 正文见 [`super`]；记号、帧与上限见 [`protocol::system::supply::frame`]。

use env::{Key, Pair};
use env::{MailFail, PieToken, Wait};
use runtime::core::port;
use runtime::env::mail::{NolePie, PolePie};

use protocol::communication::establish::Endpoint;
use protocol::communication::receiver::{Receiver, RecvFail};
use protocol::communication::sender::Sender;
use protocol::system::supply::Fail;
use protocol::system::supply::frame::{BAD, Kind, OK, Order, Reply, WANT_MAX, fail_to_code};

/// 供：照单取源、授出、把记录写进 `records`。返**条数**。
/// `src_of` = 取源：`坐标 → 我手里那一枚`。固件不认识服务、也不认识设备，只认识这句话。
/// 契约：
/// - **逐条进行**：第 i 条不成即停（`Err`）。前面已经授出的**留在对端**——它们已经归
///   对端了，本层不回滚（回滚要 `Revoke`，那是另一个动作；调用方按 `code` 处置）。
/// - **形态照请求**（`VEST` / `ONLY` 都是**请求方说**的，固件不替它挑）。
pub fn supply(
    order: &Order,
    src_of: impl Fn(Key) -> Option<PieToken>,
    records: &mut [Pair; WANT_MAX],
) -> Result<usize, Fail> {
    let who = order.who();
    let n = order.len();
    for i in 0..n {
        let want = order.want(i).ok_or(Fail::Bad)?;
        let key = want.key().ok_or(Fail::Bad)?;
        let src = src_of(key).ok_or(Fail::Unknown)?;
        let access = want.access().ok_or(Fail::Bad)?;
        // 形态**照请求**（见本函数的契约那一节）。
        let form = want.policy().ok_or(Fail::Bad)?;
        let at = match want.kind().ok_or(Fail::Bad)? {
            Kind::Pole => port::ship(&PolePie::from_token(src), who, access, form),
            Kind::Nole => port::ship(&NolePie::from_token(src), who, access, form),
        }
        .map_err(|_| Fail::Denied)?;
        records[i] = Pair::new(key, at.seed());
    }
    Ok(n)
}

/// 固件的常驻循环：收一张单子 → 供 → 回一张回单。
pub fn serve(
    pier: &Endpoint,
    src_of: impl Fn(Key) -> Option<PieToken>,
    alive: impl Fn() -> bool,
    ask: &mut [u8],
) {
    /// 探活周期（毫秒）：一次超时 = 一次探活。对端活着时这一等就是**空等**。
    const WAIT_MS: usize = 1000;
    // 记录那一格：至多 [`WANT_MAX`] 条（条数不越界由 `Order` 那一侧保证）。
    let mut records = [Pair::NONE; WANT_MAX];
    let rx = Receiver::<Order>::from_token(pier.rx());
    loop {
        // **三格失败分得开**（[`RecvFail`]）：没收到 ⇒ 去探活；孔用不动了 ⇒ 收摊；解不动 ⇒ 答 `BAD`。
        let order = match rx.recv(ask, Wait::AtMost(WAIT_MS)) {
            Ok(order) => order,
            // "没收到"（`Busy` 那一族）⇒ 去探一次活；其余三格见下面两条。
            Err(RecvFail::Mail(e)) if !matches!(e, MailFail::Dead | MailFail::Denied) => {
                if !alive() {
                    return;
                }
                continue;
            }
            // **内核当场说"这一枚孔用不动了"**——不必再问 `alive()`（那是"没消息"时的替代判据，
            // 见上面 `alive` 那一段）：端点没了，这条循环没有下一步可走。
            Err(RecvFail::Mail(_)) => return,
            Err(RecvFail::Unread(len)) => {
                protocol::debug!("supply: recv unread len={len}");
                reply(pier, BAD, &[]);
                continue;
            }
        };
        let code = match supply(&order, &src_of, &mut records) {
            Ok(k) => {
                reply(pier, OK, &records[..k]);
                continue;
            }
            Err(fail) => fail_to_code(Some(fail)),
        };
        reply(pier, code, &[]);
    }
}

/// 回一张回单：**一处发**（装与发都不在这一层写字节——缓冲在这一帧的栈上）。
/// 那头还没齐（没有写端 ⇒ [`Endpoint::tx`] 是 `None`）⇒ 不发：**没有写端就发不出去**，
fn reply(pier: &Endpoint, code: u8, records: &[Pair]) {
    let Some(at_peer) = pier.tx() else {
        return;
    };
    if let Some(reply) = Reply::of(code, records) {
        // **写端跟着这一趟走**：落出作用域时等这只手被取走（`Drop`）——那位客人不来取，
        // 卡的是他自己那一趟。
        let mut tx = Sender::<Reply>::from_token(at_peer);
        let _ = tx.send(reply);
    }
}
