use crate::{ME, device};
use env::{Access, Mark, PieKind, PieToken, Policy, Wait};
use programs::driver::shared::context::{Context, Step};
use programs::driver::shared::device::{Ask, Device, Hub};
use programs::driver::shared::fail::Fail;
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

const ASK: Ask = Ask {
    class: "ns16550a",
    name: None,
    kind: PieKind::Pole,
    access: Access::FETCH_STORE,
    policy: Policy::ONLY,
};

const RX: &str = "rx";
const TX: &str = "tx";

const TX_MARK: &str = "uart-tx";

pub struct Desk {
    /// 位置；`ctx.entry` = 读口（`publish` 往它推）
    pub ctx: Context,
    pub line: Line,
    pub tx: HolePie,
    /// **那一台**（寄存器页的映射）——常驻那一圈每醒一次读它
    pub dev: Device,
}

/// 起手：铸两枚孔 → 上板 ＋ 开会话 → 上树落两枚门牌（**自证**）→ 认领设备 → 开闸 → 占线
/// 失败那几格说**步名**（`tree` / `hub` / `bond` / `list` / `claim` / `name`）
pub fn start(ms: Wait) -> Result<Desk, Fail> {
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

    // **报"答得动了"**（Setup::Ready）：牌子落了才算——装配者等它才往下起别人，于是"排在第几号"
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

/// → `rx` / `tx` 各一枚 Tile → 各查回来一遍（号 ↔ 名对得上才算那枚号是真坐标）
/// （`plate` 那一形是"一枚门牌"，而控制台是双向的 ⇒ 两枚砖同挂一块窗格下）
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
