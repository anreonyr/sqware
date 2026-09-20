//! firmware::server — **引导域那一侧**：照单取源、授出、回一张回单（常驻循环 [`serve`]）
//!
//! 正文见 [`super`]；记号、帧与上限见 [`crate::firmware::call`]。

use env::{PAIR_LEN, Pair, PieToken};
use runtime::core::port::{self, Policy};
use runtime::env::mail::{HolePie, NolePie, PolePie};

use protocol::firmware::call::{BAD, Kind, OK, Slip, WANT_MAX, code_of_fail, reply, slip_of};
use protocol::firmware::core::Fail;
use protocol::session::Pier;

/// 供：照单取源、授出、把记录写进 `records`。返**条数**。
///
/// `src_of` = 取源：`名字 → 我手里那一枚`。固件不认识服务，只认识这句话。
///
/// 契约：
/// - **逐条进行**：第 i 条不成即停（`Err`）。前面已经授出的**留在对端**——它们已经归
///   对端了，本层不回滚（回滚要 `Revoke`，那是另一个动作；调用方按 `code` 处置）。
/// - **形态照请求，唯独 `VEST` 一律剔掉**：固件不发"再授出的权"。
pub fn supply(
    slip: &Slip<'_>,
    src_of: impl Fn(&str) -> Option<PieToken>,
    records: &mut [u8],
) -> Result<usize, Fail> {
    let who = slip.who();
    let n = slip.len();
    if records.len() < n * PAIR_LEN {
        return Err(Fail::Full);
    }
    for i in 0..n {
        let want = slip.want(i).ok_or(Fail::Bad)?;
        let name = want.name().ok_or(Fail::Bad)?;
        let src = src_of(name.as_str()).ok_or(Fail::Unknown)?;
        let access = want.access().ok_or(Fail::Bad)?;
        // 形态照请求，**唯独 `VEST` 一律剔掉**（见本函数的契约）。
        let form = want.policy().ok_or(Fail::Bad)? & !Policy::VEST;
        let at = match want.kind().ok_or(Fail::Bad)? {
            Kind::Pole => port::ship(&PolePie::from_token(src), who, access, form),
            Kind::Nole => port::ship(&NolePie::from_token(src), who, access, form),
            Kind::Hole => port::ship(&HolePie::from_token(src), who, access, form),
        }
        .map_err(|_| Fail::Denied)?;
        let pair = Pair::new(name, at.seed());
        records[i * PAIR_LEN..(i + 1) * PAIR_LEN].copy_from_slice(pair_bytes(&pair));
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
/// 前条件：`ask` ≥ [`SLIP_CAP`]、`out` ≥ [`REPLY_CAP`]。
pub fn serve(
    pier: &Pier,
    src_of: impl Fn(&str) -> Option<PieToken>,
    alive: impl Fn() -> bool,
    ask: &mut [u8],
    out: &mut [u8],
) {
    /// 探活周期（毫秒）：一次超时 = 一次探活。对端活着时这一等就是**空等**。
    const WAIT_MS: usize = 1000;
    let mut records = [0u8; PAIR_LEN * WANT_MAX];
    loop {
        let Ok(n) = pier.pull(ask, WAIT_MS) else {
            if !alive() {
                return;
            }
            continue;
        };
        let code = match ask.get(..n).and_then(slip_of) {
            Some(slip) => match supply(&slip, &src_of, &mut records) {
                Ok(k) => {
                    if let Some(frame) = reply(out, OK, &records[..k * PAIR_LEN]) {
                        let _ = pier.post(frame);
                    }
                    continue;
                }
                Err(fail) => code_of_fail(fail),
            },
            None => BAD,
        };
        if let Some(frame) = reply(out, code, &[]) {
            let _ = pier.post(frame);
        }
    }
}

/// 一条记录的字节：**名字块 + 句柄**——尺寸由 `Pair` 自己锁死，这里只是一次只读的
/// 字节视图（`Pair` 是 `repr(C)`，内容即线格式）。
pub(crate) fn pair_bytes(pair: &Pair) -> &[u8; PAIR_LEN] {
    // SAFETY: `Pair` 是 `repr(C)`、尺寸由编译期断言等于 `PAIR_LEN`，只读解释为字节安全。
    unsafe { &*(pair as *const Pair).cast::<[u8; PAIR_LEN]>() }
}
