//! supply::server — **引导域那一侧**：照单取源、授出、回一张回单（常驻循环 [`serve`]）
//!
//! 正文见 [`super`]；记号、帧与上限见 [`protocol::system::supply::frame`]。

use env::{MailFail, PieToken, Wait};
use env::{Key, Pair};
use runtime::core::port;
use runtime::env::mail::{NolePie, PolePie};

use protocol::system::supply::frame::{BAD, Kind, OK, Order, Reply, WANT_MAX, fail_to_code};
use protocol::system::supply::Fail;
use protocol::communication::establish::Endpoint;
use protocol::communication::receiver::{Receiver, RecvFail};
use protocol::communication::sender::Sender;

/// 供：照单取源、授出、把记录写进 `records`。返**条数**。
///
/// `src_of` = 取源：`坐标 → 我手里那一枚`。固件不认识服务、也不认识设备，只认识这句话。
///
/// 契约：
/// - **逐条进行**：第 i 条不成即停（`Err`）。前面已经授出的**留在对端**——它们已经归
///   对端了，本层不回滚（回滚要 `Revoke`，那是另一个动作；调用方按 `code` 处置）。
/// - **形态照请求**（`VEST` / `ONLY` 都是**请求方说**的，固件不替它挑）。
///
/// **照实记（"唯独 `VEST` 一律剔掉"那条规矩退了）**：这一句从前是 `& !Policy::VEST`，理由
/// 写的是"**固件不发'再授出的权'**"——它说得通的前提是**每一台驱动各自向固件领自己那一份**：
/// 那种世界里"把一枚设备再授给别人"没有客人，只有多一层转手的风险。
///
/// 这一刀把那个前提换掉了：整台机器**只经一条路**领出去——装配者领给**设备账那一台**
/// （`Setup::Machine`），而那一位的**全部工作就是把每一台再授给它认领的那台驱动**。它手里那
/// 一份不带 `VEST` 就一台都交不出去（`accord` 一句"源枚不持 `VEST`"当场拒）。故那一句删掉：
/// **"这一枚能不能再授出"从此是请求方那一格说了算**，而**独占**那一半并没有放开——
/// 设备 `reg` 段那一枚带 `ONLY`，内核按"`ONLY` 必须一致 ＋ 一枚至多一个 heir"执行：
/// 设备账一次只能把一台交给一位（见 `protocol::driver::hub` 那一条照实记）。
///
/// **照实记（"记录缓冲装不下"那一格退场）**：那一格从前是
/// `records.len() < n × PAIR_LEN ⇒ Full`——今天 `records` 恰好是 [`WANT_MAX`] 条，而条数不越界
/// 由 [`Order`] 那一侧（`fetch`）保证 ⇒ 装不下**不可表达**，那一格没了。
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
///
/// `alive` = "客户还在吗"。**为什么还需要它**（照实记：这一格收窄过一次）：本域读的那一枚孔
/// **命随本端**（对端退出时，内核那条寿命边封的是**对端开的那几枚**）⇒ "对端没了"在这条路上
/// **报不出来**，读超时也不能当作它没了。故读有上界地等，超时只探一次活（非阻塞），探出没了
/// 就收场；对端活着时它就是在等，不占核。
///
/// **它现在只管"没消息"那一支**：内核当场说"这一枚孔用不动了"时走 `Dead` / `Denied` 那一格，
/// 那一支直接收摊、**不再问 `alive`**（端点没了，没有下一步可走）——两条判据不再互相顶替。
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
            Err(RecvFail::Unread) => {
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
///
/// 那头还没齐（没有写端 ⇒ [`Endpoint::tx`] 是 `None`）⇒ 不发：**没有写端就发不出去**，
/// 不猜、不空转（与从前同一条口径）。
fn reply(pier: &Endpoint, code: u8, records: &[Pair]) {
    let Some(at_peer) = pier.tx() else {
        return;
    };
    if let Some(reply) = Reply::of(code, records) {
        // **装不上那一格按构造到不了**（`Buf` 由本族 `Message` 自己给，见 `Sender::send`
        // 的照实记）：`.ok()` 显式落地一个到不了的点，不是吞错。
        let _ = Sender::<Reply>::from_token(at_peer)
            .send(reply, Wait::Forever)
            .ok();
    }
}
