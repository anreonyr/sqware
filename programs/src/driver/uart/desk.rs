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
//! 自己走一趟（`rtc` 仍走 `enter`）。
//!
//! 写的判据与读同一条：**一次写 = 一条完整的字**——客人推来的**一条消息**就是要写出去的
//! 全部字节；本域不拆、不并、不添字（换行由客人补，见 `programs/src/user/canonical/main.rs`）。

use super::ME;
use env::{Key, Mark, Name, PieToken, Wait};
use programs::driver::context::{Context, Step};
use programs::driver::fail::Fail;
use programs::program::uart::E_UART;
use protocol::debug;
use protocol::driver::DIR;
use protocol::driver::line::client::Line;
use protocol::system::board::ENTRY_MARK;
use protocol::system::operator::client as operator;
use protocol::system::operator::client::Mine;
use protocol::system::operator::{EntryId, Rule};
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
/// 判据与 [`Context::enter`] 那一趟同一条：**这一域没登记上就不该活着** ⇒ 不成即断言。
/// 两枚砖**都声明归本域**（`probe-owner` 顶的就是这一格）。
///
/// **本台借一面 `Face` 的视图**（[`operator::Face::from`]）：它要在**同一条会话**上落两枚门牌，
/// 而会话本身仍归 [`Context`]。两枚砖**都声明归本域**（`probe-owner` 顶的就是这一格）。
fn plate(ctx: &Context, rx: PieToken, tx: PieToken, ms: Wait) {
    let (Ok(dev), Ok(me), Ok(rx_name), Ok(tx_name)) =
        (Name::new(DIR), Name::new(ME), Name::new(RX), Name::new(TX))
    else {
        debug!("{ME}: tree: bad name");
        return;
    };
    let tree = operator::Face::from(&ctx.session);
    let root = tree.root();
    // 分目录两趟：第一趟 `/device`（幂等），第二趟本域那块 `/device/uart`。
    let opened_dir = root.open(dev, ms);
    let (part, _dir) = match &opened_dir {
        Ok(at) => (Ok(()), at.id()),
        Err(fail) => (Err(*fail), EntryId::new(0)),
    };
    let opened_pane = match &opened_dir {
        Ok(at) => at.open(me, ms),
        Err(fail) => Err(*fail),
    };
    let (pane, at) = match &opened_pane {
        Ok(pane) => (Ok(()), pane.id()),
        Err(fail) => (Err(*fail), EntryId::new(0)),
    };
    let _ = at;
    // 两枚砖**都声明归本域**（`probe-owner` 顶的就是这一格）。
    let laid_rx = match &opened_pane {
        Ok(pane) => pane
            .bind(rx_name, rx, Rule::Public, Mine::Yes, ms)
            .map(|e| e.id()),
        Err(fail) => Err(*fail),
    };
    let laid_tx = match &opened_pane {
        Ok(pane) => pane
            .bind(tx_name, tx, Rule::Public, Mine::Yes, ms)
            .map(|e| e.id()),
        Err(fail) => Err(*fail),
    };
    // 自证：按号问名——**一次问完两格**（`name` 那一手既答名、也证明那一号还在）。
    let found_rx = match &laid_rx {
        Ok(id) => root.name(*id, ms),
        Err(fail) => Err(*fail),
    };
    let found_tx = match &laid_tx {
        Ok(id) => root.name(*id, ms),
        Err(fail) => Err(*fail),
    };
    let named_rx = found_rx.as_ref().ok();
    let named_tx = found_tx.as_ref().ok();
    let (got_rx, got_tx) = (found_rx.is_ok(), found_tx.is_ok());
    debug!(
        "{ME}: tree part={part:?} pane={pane:?} rx={:?} tx={:?} find_rx={found_rx:?} find_tx={found_tx:?} got={got_rx},{got_tx} pname={},{}",
        laid_rx.as_ref().map(|id| id.get()),
        laid_tx.as_ref().map(|id| id.get()),
        named_rx.map(|n| n.as_str()).unwrap_or("-"),
        named_tx.map(|n| n.as_str()).unwrap_or("-"),
    );
    assert!(part.is_ok());
    assert!(pane.is_ok());
    assert!(laid_rx.is_ok());
    assert!(laid_tx.is_ok());
    assert!(found_rx.is_ok());
    assert!(found_tx.is_ok());
    assert!(got_rx && got_tx);
    assert_eq!(named_rx.map(|n| n.as_str()), Some(RX));
    assert_eq!(named_tx.map(|n| n.as_str()), Some(TX));
}
