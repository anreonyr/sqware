//! control::enroll — **装配那一半**：先立账（`enlist`），后入册（`enroll`）。
//! 两条都是"机器对整张单做的事"，与具体是哪一台无关：
//!   - [`Control::enlist`]：登记一行——**怎么算起来了**由这一行的 `setup` 推出
//!     （有通道 ⇒ `Announce::Channel`；否则放行即起来）。**先立整张账，再逐条起**。
//!   - [`Control::enroll`]：**把这一台机器的全部可领之物交进某一台**（`Setup::Machine` 那一格
//!     声明的）：按机器自述枚举全机 → 逐条授出（门闩在本域手里）→ 一整段推进它那条通道。

use alloc::string::String;
use alloc::string::ToString;
use env::{Pair, Wait};
use protocol::debug;

use env::{Access, Key, PieKind, Mark, Policy};
use protocol::service::hub::{ENROLL_MAX, Enroll};
use runtime::core::res::port;
use runtime::env::mail::{NolePie, PolePie};

use crate::system::common::life::table::Announce;

use super::{BOOT_MS, Control, Error,  Service};
use crate::system::Assembly;
use crate::unit::{Setup, UnitFile};

impl Control {
    /// **登记一行**：只知道名字、它"怎么算起来"、以及**谁结束它**——此刻还没有身子（`spawn` 才挂）。
    /// **"怎么算起来"由 `setup` 推出**：有通道 ⇒ [`Announce::Channel`]（它起来时会交回
    pub fn enlist(&mut self, program: &UnitFile) -> Result<(), Error> {
        let name = program.name().to_string();
        let restart = program.relation.restart.ok_or(Error::Step("no ending"))?;
        self.table
            .register(name, announce_of(program.supply()), restart)
            .map_err(|_| Error::Table)
    }

    /// **放行 + 入册**（装配那一相的后半）：**次序是硬的**——物料要落到它交回的那条路上，
    /// 故入册只能在放行之后（[`Control::start`] 之后才 [`Control::enroll`]）。
    pub fn launch(
        &mut self,
        program: &UnitFile,
        name: String,
        service: &mut Service,
    ) -> Result<(), Error> {
        // **一、放行**（不等就绪）。
        self.start(name.as_str(), service)?;
        // **二、递物料**（只对声明了 `Setup::Machine` 的那一台；别的台这一步是空转）。
        // **等就绪**由调用方接在它该在的位置（见本手的注）。
        if let Some(load) = program
            .demand
            .supply
            .iter()
            .find(|s| s.machine())
            .map(Setup::channel)
        {
            self.enroll(name, service, load)?;
        }
        Ok(())
    }

    /// **入册**：把**这台机器的全部可领之物**交进收方（那一条声明了 `Setup::Machine` 的通道）。
    /// 前置：它**已经起来**（[`Control::start`] 之后）——通道那一头才认得上，记录也才落得进去。
    /// 三步：
    /// 1. **枚举全机**（[`crate::system::common::machine::Machine::devices`]）＋ 那两件按**已知坐标**的
    ///    （设备树本体 / 门铃——它们不在树里，没有"哪一类"可判）；
    /// 2. **逐条授出**（门闩在本域手里，直接 `port::ship` 给**它**）；
    /// 3. **一整段推过去**（[`Enroll`]：条数 ＋ 那几条 `Pair` 记录，一个字节都不翻译）。
    /// **第一条恒是设备树本体**：收方要**先**把树读一遍，才知道哪一条记录是哪一台
    /// （名 / 类 / 线）。次序即契约。
    pub fn enroll(
        &mut self,
        name: String,
        service: &mut Service,
        load: &'static str,
    ) -> Result<(), Error> {
        let (task, channels) = service;
        // 递单走**第一条**通道（`setup` 里 `Machine` 那一格装出来的那一条，见 `connect_all`）。
        let Some(link) = channels.first_mut() else {
            return Err(Error::Step("no channel"));
        };
        // **先认下它交回的那一枚**（放行之后它第一件事就是铸这一枚）：`push` 要的是**它表里**
        // 那个号（`Endpoint::tx`），而那要本域先 `claim` 一次。这是"放行 → 认通道 → 递物料"
        // 那三步里的中间一步（次序见 [`Control::launch`]）。
        if !link.claim(*task, Mark::of(load), Wait::AtMost(BOOT_MS)) {
            return Err(Error::Step("no channel"));
        }
        // **那头齐了没有**：写端在不在。
        let Some(tx) = link.tx() else {
            return Err(Error::Step("no channel"));
        };

        // 一、这一段要有哪几样（收方那一台要**先**把树读一遍，故第一条是树本体）。
        let devices = self
            .machine
            .devices()
            .ok_or(Error::Step("no room for devices"))?;
        let total = devices.len() + 2;
        if total > ENROLL_MAX {
            return Err(Error::Step("too many devices"));
        }

        // 二、逐条授出：本域持门闩，**直接授给客人**；取不到源 / 授不出的那一条跳过并记一行读数。
        // **形态照源枚**：设备那几段带 `ONLY`（内核就是那么发的：一枚门闩只许一个使用者），
        // 树与门铃不带。**四条都带 `VEST`**：收方（设备账那一台）的全部工作就是**再授出**
        // （把每一台交到它认领者手里）——不带 `VEST` 它就一台都交不出去。
        // 坐标那一格：树与门铃按**已知坐标**要（它们不在树里），设备按**区**要（那是内核造门闩
        // 的坐标）。
        let mut records = [Pair::NONE; ENROLL_MAX];
        let mut got = 0usize;
        let mut put = |key: Key, kind: PieKind, access: Access, policy: Policy| {
            let shipped = self.accounts.token(key).and_then(|src| match kind {
                PieKind::Pole => port::ship(&PolePie::from_token(src), *task, access, policy).ok(),
                PieKind::Nole => port::ship(&NolePie::from_token(src), *task, access, policy).ok(),
            });
            match shipped {
                Some(seat) => {
                    records[got] = Pair::new(key, seat.seed());
                    got += 1;
                }
                // 授不出的那一台：**一行读数**，这一台不在册上。
                None => debug!(
                    "system: enroll {} skipped {:#x}",
                    name.as_str(),
                    key.base().unwrap_or(0)
                ),
            }
        };
        put(Key::dtb(), PieKind::Pole, Access::FETCH, Policy::VEST);
        put(Key::irq(), PieKind::Nole, Access::FETCH, Policy::VEST);
        for device in &devices {
            put(
                device.key,
                PieKind::Pole,
                Access::FETCH_STORE,
                Policy::VEST | Policy::ONLY,
            );
        }

        // 三、一整段推过去。
        let Some(enroll) = Enroll::of(&records[..got]) else {
            return Err(Error::Step("too many devices"));
        };
        // **递出即返回**：等它下线由 `Control` 那一格写端担着（见它的注；`send` 里先收口上一手）。
        self.out = protocol::communication::sender::Sender::<Enroll>::from_token(tx);
        if self.out.send(enroll).is_err() {
            return Err(Error::Step("no channel"));
        }
        debug!("system: enrolled {} supplies for {}", got, name.as_str());
        Ok(())
    }
}

/// **生命这一轴在装配那一趟里的那一手**：等这一台的凭据交齐（读它 `setup` 那几格）。
pub fn await_ready(
    assembly: &mut Assembly,
    program: &UnitFile,
    service: &mut Service,
) -> Result<(), &'static str> {
    assembly
        .control
        .ready(program.name().to_string(), service, program.supply())
        .map_err(|e| e.said())
}

/// **怎么算"它起来了"**：由这一行的 `setup` 推出（见 [`Control::enlist`]）。
fn announce_of(supply: &[Setup]) -> Announce {
    if supply.is_empty() {
        Announce::None
    } else {
        Announce::Channel
    }
}

/// **装通道**（"配"那一相）：按这一台 `setup` 里那几格逐条装上——**记号 = 通道名**，
/// 放行后按同一个记号逐条认领（[`service::ready`](super::service::ready)）。
/// **自由函数**：它只碰通道，不碰 `Control` 的任何一格（与 [`connect`](super::connect) 那一手
/// 同一句正文——"只碰通道"的那一层做成方法就是白加的壳）。两处叫它：装配那一趟
/// （[`crate::system::Assembly::assemble`]）与线上那条 [`Control::release`]。
pub fn connect_all(program: &UnitFile, service: &mut Service) -> Result<(), Error> {
    for s in program.supply() {
        for ch in [Some(s.channel()), s.ready()].into_iter().flatten() {
            service
                .1
                .try_reserve(1)
                .map_err(|_| Error::Step("no room for channels"))?;
            let channel = super::connect(service.0, ch)?;
            service.1.push(channel);
        }
    }
    Ok(())
}
