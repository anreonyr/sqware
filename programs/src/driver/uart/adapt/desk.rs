use crate::device;
use env::{Access, PieKind, Policy, Wait};
use programs::driver::shared::context::{Context, Step};
use programs::driver::shared::device::{Ask, Device, Hub};
use programs::driver::shared::fail::Fail;
use programs::driver::uart::core::frame::{Bytes, ME, RX, TX};
use programs::unit::uart::E_UART;
use ipc::rack::{Mode, Rack, Reader, Writer};
use protocol::debug;
use protocol::driver::line::Line;
use system_client::control::Scope;
use system_client::operator::Permit;
use system_client::operator::client as operator;
use env::unit;

const ASK: Ask = Ask {
    class: "ns16550a",
    name: None,
    kind: PieKind::Pole,
    access: Access::FETCH_STORE,
    policy: Policy::ONLY,
};

pub struct Desk {
    /// 这一域与树的那条会话。**它只为"持着"而存在**（下划线起头，与 `rack::Writer::_dock`
    /// 同一规）：本域起手之后不再跟树说话，但会话要活到本域走完——放了就是撤那条路。
    pub _ctx: Context,
    pub line: Line,
    /// **读口那一具架**（设备排空 → 落进它的环）。本域是**写端**；落砖时把这一枚页交出去。
    /// 同上：**它只为"持着"映射而存在**——`rx_w` 只借 `View`，页的映射归这一格。
    pub _rx: Rack<Bytes>,
    pub rx_w: Writer<Bytes>,
    /// **写口那一具架**（人敲的字落在它的环里等本域取）。本域是**读端**；`mode` 归对面（它写）。
    pub tx: Rack<Bytes>,
    pub tx_r: Reader<Bytes>,
    /// **那一台**（寄存器页的映射）——常驻那一圈每醒一次读它
    pub dev: Device,
}

/// 起手：上板 ＋ 开会话 → 认领设备 → 开闸 → 占线 → **开两具架** → 上树落两枚门牌（自证）→
/// 报 `Ready`。失败那几格说**步名**（`tree` / `desk` / `line` …）。
pub fn start(ms: Wait) -> Result<Desk, Fail> {
    let ctx = Context::open(unit::sire(), ms).map_err(|s| {
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

    // **两具架**：一枚页就是一具完整的架（页上那一位即铃）⇒ 树上一格门牌正好挂一具。
    // rx 那一侧的 `mode` 是本端（写端）的规矩；tx 那一侧本端只读，`mode` 归对面自选。
    let rx = Rack::<Bytes>::open(Mode::Oldest).map_err(|_| Fail::at(E_UART, "desk"))?;
    let rx_w = rx.writer();
    let tx = Rack::<Bytes>::open(Mode::Oldest).map_err(|_| Fail::at(E_UART, "desk"))?;
    let tx_r = tx.reader();

    plate(&rx, &tx, ms);

    // **报"答得动了"**（Setup::Ready）：牌子落了才算——装配者等它才往下起别人，于是"排在第几号"
    let _ = ipc::session::establish::endpoint(
        env::unit::sire(),
        env::Mark::of(programs::unit::READY),
        env::Wait::POLL,
    );

    Ok(Desk {
        _ctx: ctx,
        line,
        _rx: rx,
        rx_w,
        tx,
        tx_r,
        dev,
    })
}

/// → `rx` / `tx` 各一枚 Tile（**各是一具架的页**）→ 各查回来一遍（号 ↔ 名对得上才算真坐标）
/// 送出去的是 `Rack::ship()` 交给对端的**那一枚页**：客人拿它既能映页、又能等页上那一位。
fn plate(rx: &Rack<Bytes>, tx: &Rack<Bytes>, ms: Wait) {
    let client = system_client::control::publication::Client::injected()
        .expect("uart: publication entry");
    for (name, entry) in [(RX, rx.ship()), (TX, tx.ship())] {
        let target = system_client::control::publication::Target::Service {
            scope: Scope::Driver,
            group: "uart".into(),
            name: name.into(),
        };
        client
            .publish(target, entry, Permit::Public, ms)
            .expect("uart: publication");
    }
}
