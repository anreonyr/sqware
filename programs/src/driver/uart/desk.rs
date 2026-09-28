//! uart::desk — **服务台**：本域对客人那两面的落点，以及"把一条字写出去"这一手。
//!
//! ```text
//!   起手  铸两枚孔 → 上板 ＋ 开会话 → `/device/uart` 那块 Pane 下落两枚 Tile → 占线
//!   常驻  从 `tx` 取一条字 ⇒ 原样写进设备（[`put`](crate::uart::put)）
//! ```
//!
//! # 照实记（这一台为什么不是一枚门牌）
//!
//! 控制台是**双向**的：读（本域排空出来的一批，交给 `canonical`）与写（客人交来的一条字）。
//! 树上一枚 Tile 只挂一枚 Pie，故本域那一格从一枚砖变成**一块 Pane**，两枚门牌各占一枚
//! （用户裁定 `/device/uart/{rx,tx}`）。`Context::enter` 那一趟是"一枚门牌"那一形，故本域
//! 交的路长一段、牌两枚；**那一趟的机制与 `enter` 同住 [`bridge::land`]**（`rtc` 仍走 `enter`）。
//!
//! 写的判据与读同一条：**一次写 = 一条完整的字**——客人推来的**一条消息**就是要写出去的
//! 全部字节；本域不拆、不并、不添字（换行由客人补，见 `programs/src/user/canonical/main.rs`）。

use super::ME;
use env::{Key, Mark, PieToken, Wait};
use programs::driver::context::{Context, Step};
use programs::driver::fail::Fail;
use programs::program::uart::E_UART;
use programs::system::operator::bridge;
use protocol::debug;
use protocol::driver::DIR;
use protocol::driver::line::client::Line;
use protocol::system::board::ENTRY_MARK;
use protocol::system::operator::client as operator;
use protocol::system::operator::client::Mine;
use runtime::env::mail;
use runtime::env::mail::HolePie;
use runtime::env::unit as utask;

/// 两枚门牌在 `/device/uart` 下的名字：`rx` = 读口（本域推、客人取），`tx` = 写口（客人推、本域取）。
const RX: &str = "rx";
const TX: &str = "tx";

/// 写口那一枚孔的记号：**本域自己铸的**，记号只要本域认得（客人按名字从树上拿那一枚）。
const TX_MARK: &str = "uart-tx";

/// 本域在系统里的位置：`Context`（会话 ＋ **读口**那一枚）＋ 那条线 ＋ 写口。
pub struct Desk {
    /// 位置；`ctx.entry` = 读口（`publish` 往它推）。
    pub ctx: Context,
    /// 本域那条线（路由者那面：它说"线响了"，本域说"我排空了"）。
    pub line: Line,
    /// **写口**：客人往这里推一条字，本域常驻里取走写进设备。
    pub tx: HolePie,
}

/// 起手：铸两枚孔 → 上板 ＋ 开会话 → 上树落两枚门牌（**自证**）→ 占线。
///
/// 失败那几格说**步名**（`board` / `tree` / `line`），与 [`Context::enter`] 同一口径。
pub fn start(key: Key, ms: Wait) -> Result<Desk, Fail> {
    // 门牌**先解**：读口用板那枚统一记号，写口是本域自己的一枚。
    let rx = mail::unseal_hole(ENTRY_MARK).map_err(|_| Fail::at(E_UART, "tree"))?;
    let tx = mail::unseal_hole(Mark::of(TX_MARK)).map_err(|_| Fail::at(E_UART, "tree"))?;
    let ctx = Context::join(rx, utask::sire(), ms).map_err(|s| {
        Fail::at(
            E_UART,
            match s {
                Step::Board => "board",
                Step::Tree => "tree",
            },
        )
    })?;
    plate(&ctx, rx, tx, ms);
    let line = ctx.line(key, ms).map_err(|_| Fail::at(E_UART, "line"))?;
    debug!("{ME}: line occupied");
    Ok(Desk {
        ctx,
        line,
        tx: HolePie::from_token(tx),
    })
}

/// 落两枚门牌并自证：`/device`（幂等——别的驱动也在它下面）→ `/device/uart`（本域那块 Pane）
/// → `rx` / `tx` 各一枚 Tile → 各查回来一遍（号 ↔ 名对得上才算那枚号是真坐标）。
///
/// **这一趟本身住在 `bridge::land`**：与 `Context::plate` 同一趟，只是**路长一段、牌两枚**
/// （[`Context::enter`] 那一形是"一枚门牌"，而控制台是双向的 ⇒ 两枚砖同挂一块窗格下）。
///
/// **照实记（自证口径跟着四家统一，多两趟往返）**：原先这里只问一句 `name`（"一次问完两格"：
/// 既答名、也证明那一号还在），收进 `land` 之后四家同一条口径——`token`（路译得回 ＋ 那一枚门闩
/// 取得回来）**再加** `name`。故两枚门牌各多两问（`tile` ＋ `token`），boot 期四问、不在稳态。
/// 落在这：**一处口径**比**两趟往返**值。
///
/// 判据与 [`Context::plate`] 同一条：**这一域没登记上就不该活着** ⇒ 不成即断言。
/// 两枚砖**都声明归本域**（`probe-owner` 顶的就是这一格）。
///
/// **本台借一面 `Face` 的视图**（[`operator::Face::from`]）：它要在**同一条会话**上落两枚门牌，
/// 而会话本身仍归 [`Context`]。
fn plate(ctx: &Context, rx: PieToken, tx: PieToken, ms: Wait) {
    let tree = operator::Face::from(&ctx.session);
    let list = [(RX, rx), (TX, tx)];
    let plated = bridge::land(&tree, ME, &[DIR, ME], Mine::Yes, &list, ms);
    assert_eq!(plated.len(), 2, "{ME}: tree: road");
    for (one, want) in plated.iter().zip([RX, TX]) {
        assert!(one.land.is_ok(), "{ME}: tree: land {want}");
        assert!(one.find.is_ok(), "{ME}: tree: find {want}");
        assert_eq!(
            one.named.as_ref().map(|name| name.as_str()),
            Some(want),
            "{ME}: tree: name {want}"
        );
    }
}
