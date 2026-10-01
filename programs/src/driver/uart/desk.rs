//! uart::desk — **服务台**：本域对客人那两面的落点，以及"把一条字写出去"这一手。
//! ```text
//!   起手  铸两枚孔 → 上板 ＋ 开会话 → `/svc/drv/uart` 那块 Pane 下落两枚 Tile
//!         → **认领设备**（设备账那一台）→ 开闸 → 占线
//!   常驻  从 `tx` 取一条字 ⇒ 原样写进设备（[`put`](crate::uart::put)）
//! ```

use super::{ME, device};
use env::{Access, PieKind, Mark, PieToken, Policy, Wait};
use programs::driver::context::{Context, Step};
use programs::driver::device::{Ask, Device, Hub};
use programs::driver::fail::Fail;
use programs::service::operator::bridge;
use programs::unit::uart::E_UART;
use protocol::debug;
use protocol::driver;
use protocol::driver::ENTRY_MARK;
use protocol::driver::line::client::Line;
use protocol::service::operator::Permit;
use protocol::service::operator::client as operator;
use protocol::service::operator::client::Mine;
use runtime::env::mail;
use runtime::env::mail::HolePie;
use runtime::env::unit as utask;

/// 本域要认的那一台：**那一台 `ns16550a`**（类 ＋ 独占的读写真）。
const ASK: Ask = Ask {
    class: "ns16550a",
    name: None,
    kind: PieKind::Pole,
    access: Access::FETCH_STORE,
    policy: Policy::ONLY,
};

/// 两枚门牌在 `/svc/drv/uart` 下的名字：`rx` = 读口（本域推、客人取），`tx` = 写口（客人推、本域取）。
const RX: &str = "rx";
const TX: &str = "tx";

/// 写口那一枚孔的记号：**本域自己铸的**，记号只要本域认得（客人按名字从树上拿那一枚）。
const TX_MARK: &str = "uart-tx";

/// 本域在系统里的位置：`Context`（会话 ＋ **读口**那一枚）＋ 那条线 ＋ 写口 ＋ 那台设备。
pub struct Desk {
    /// 位置；`ctx.entry` = 读口（`publish` 往它推）。
    pub ctx: Context,
    /// 本域那条线（路由者那面：它说"线响了"，本域说"我排空了"）。
    pub line: Line,
    /// **写口**：客人往这里推一条字，本域常驻里取走写进设备。
    pub tx: HolePie,
    /// **那一台**（寄存器页的映射）——常驻那一圈每醒一次读它。
    pub dev: Device,
}

/// 起手：铸两枚孔 → 上板 ＋ 开会话 → 上树落两枚门牌（**自证**）→ 认领设备 → 开闸 → 占线。
/// 失败那几格说**步名**（`tree` / `hub` / `bond` / `list` / `claim` / `name`）。
pub fn start(ms: Wait) -> Result<Desk, Fail> {
    // 门牌**先解**：读口用板那枚统一记号，写口是本域自己的一枚。
    let rx = mail::unseal_hole(ENTRY_MARK).map_err(|_| Fail::at(E_UART, "tree"))?;
    let tx = mail::unseal_hole(Mark::of(TX_MARK)).map_err(|_| Fail::at(E_UART, "tree"))?;
    let ctx = Context::join(rx, utask::sire(), ms).map_err(|s| {
        Fail::at(
            E_UART,
            match s {
                Step::Tree => "tree",
            },
        )
    })?;
    let tree = operator::Face::from(&ctx.session);
    let hub = Hub::find(&tree, E_UART, ms)?;
    let deed = hub.claim(&tree, &ASK, E_UART, ms)?;
    debug!("{ME}: claimed {}", deed.name.as_str());
    let dev = Device::open(deed.token).map_err(|_| Fail::at(E_UART, "device open failed"))?;
    device::arm_rx(dev.view());
    let line = ctx
        .line(deed.line, ms)
        .map_err(|_| Fail::at(E_UART, "line"))?;
    debug!("{ME}: line occupied");

    plate(&ctx, rx, tx, ms);

    // **报"答得动了"**（`Setup::Ready`）：牌子落了才算——装配者等它才往下起别人，于是"排在第几号"
    let _ = protocol::communication::establish::endpoint(
        runtime::env::unit::sire(),
        env::Mark::of(programs::unit::READY),
        env::Wait::POLL,
    );

    Ok(Desk {
        ctx,
        line,
        tx: HolePie::from_token(tx),
        dev,
    })
}

/// 落两枚门牌并自证：`/svc/drv`（幂等——别的驱动也在它下面）→ `/svc/drv/uart`（本域那块 Pane）
/// → `rx` / `tx` 各一枚 Tile → 各查回来一遍（号 ↔ 名对得上才算那枚号是真坐标）。
/// **这一趟本身住在 `bridge::land`**：与 `Context::plate` 同一趟，只是**路长一段、牌两枚**
/// （`plate` 那一形是"一枚门牌"，而控制台是双向的 ⇒ 两枚砖同挂一块窗格下）。
fn plate(ctx: &Context, rx: PieToken, tx: PieToken, ms: Wait) {
    let tree = operator::Face::from(&ctx.session);
    let list = [(RX, rx), (TX, tx)];
    let road = driver::ROAD.try_join(ME).expect("uart: tree: road");
    let plated = bridge::land(&tree, ME, &road, Mine::Yes, Permit::Unset, &list, ms);
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
