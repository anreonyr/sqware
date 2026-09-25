//! supply::server — **引导域那一侧**：照单取源、授出、回一张回单（常驻循环 [`serve`]）
//!
//! 正文见 [`super`]；记号、帧与上限见 [`contract::driver::supply::frame`]。

use env::Wait;
use env::{PieToken};
use plan::{Key, Pair};
use runtime::core::port::{self, Policy};
use runtime::env::mail::{NolePie, PolePie};

use contract::driver::supply::frame::{BAD, Kind, OK, Order, Reply, WANT_MAX, fail_to_code};
use protocol::driver::supply::core::Fail;
use protocol::session::Pier;
use protocol::session::slip::Slip;
use protocol::session::slip::Land;

/// 供：照单取源、授出、把记录写进 `records`。返**条数**。
///
/// `src_of` = 取源：`坐标 → 我手里那一枚`。固件不认识服务、也不认识设备，只认识这句话。
///
/// 契约：
/// - **逐条进行**：第 i 条不成即停（`Err`）。前面已经授出的**留在对端**——它们已经归
///   对端了，本层不回滚（回滚要 `Revoke`，那是另一个动作；调用方按 `code` 处置）。
/// - **形态照请求，唯独 `VEST` 一律剔掉**：固件不发"再授出的权"。
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
        // 形态照请求，**唯独 `VEST` 一律剔掉**（见本函数的契约）。
        let form = want.policy().ok_or(Fail::Bad)? & !Policy::VEST;
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
/// `alive` = "客户还在吗"。**为什么需要它**：本域读的那一枚孔**命随本端**（对端退出时
/// 内核的寿命边封的是**对端开的那几枚**）⇒ 读超时不能当作"它没了"。故读有上界地等，
/// 超时只探一次活（非阻塞），探出没了就收场。对端活着时它就是在等，不占核。
///
/// 不碰策略：只按单子发货，形态剔 `VEST`。
///
/// 前条件：`ask` ≥ [`ORDER_CAP`]（**收帧那一只由调用方给**——理由见 `Slip::land` 的照实记：
/// 门那一侧收帧拿的是载体那一页；这里给的是本族最长那一只）。
pub fn serve(
    pier: &Pier,
    src_of: impl Fn(Key) -> Option<PieToken>,
    alive: impl Fn() -> bool,
    ask: &mut [u8],
) {
    /// 探活周期（毫秒）：一次超时 = 一次探活。对端活着时这一等就是**空等**。
    const WAIT_MS: usize = 1000;
    // 记录那一格：至多 [`WANT_MAX`] 条（条数不越界由 `Order` 那一侧保证）。
    let mut records = [Pair::NONE; WANT_MAX];
    let slip = Slip::<Order>::seal(pier.hole());
    loop {
        // **两格失败分得开**（[`Land`]）：没收到 ⇒ 去探活；收到了解不动 ⇒ 答一句 `BAD`。
        let order = match slip.land(ask, Wait::AtMost(WAIT_MS)) {
            Ok(order) => order,
            Err(Land::Expired) => {
                if !alive() {
                    return;
                }
                continue;
            }
            Err(Land::Unread) => {
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

/// 回一张回单：**一处发**（装与发都不在这一层写字节——缓冲是船台自己那只）。
///
/// 泊位那头还没齐（`at_peer` 空）⇒ 不发：与从前 `Pier::post` 自己那一格同一个意思
/// （"没有写端就发不出去"，不猜、不空转）。
fn reply(pier: &Pier, code: u8, records: &[Pair]) {
    let Some(at_peer) = pier.at_peer() else {
        return;
    };
    if let Some(reply) = Reply::of(code, records) {
        let _ = Slip::<Reply>::seal(at_peer).load(reply).ship();
    }
}
