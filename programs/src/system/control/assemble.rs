//! control::assemble — **装配那一半**：先立账（`enlist`），后入册（`enroll`）。
//!
//! 两条都是"机器对整张单做的事"，与具体是哪一台无关：
//!   - [`Control::enlist`]：登记一行——**怎么算起来了**由这一行的 `setup` 推出
//!     （有通道 ⇒ `Announce::Channel`；否则放行即起来）。**先立整张账，再逐条起**。
//!   - [`Control::enroll`]：**把这一台机器的全部可领之物交进某一台**（`Setup::Machine` 那一格
//!     声明的）：按机器自述枚举全机 → 逐段向引导域领 → 一整段推进它那条通道。
//!
//! **照实记（`wire` 那一手退场了）**：这一份从前还有第三条——`Control::wire`：按收方那张需求
//! 单（`Setup::Need`）逐条 **settle** 坐标、向引导域领、再按位次把记录推给收方。那一条路
//! **整个退了**：设备那一轴今天由**设备账那一台**（`system/hub`）在运行期回答"这台上哪条线"、
//! "这一段区归谁"，而取法由驱动自己跑一趟（`bond` → 列册 → 找门 → 认领，见
//! [`protocol::driver::hub`]）⇒ 装配者不必再替谁认设备、也不必再读树认类。
//!
//! **留下的是同一只手，换了收方**：递单那一半从"很多台各一张单"收成"**一台领全机**"
//! （`enroll`），因为"谁回答'这台是哪一段区'"从装配者搬进了设备账。
//!
//! **本域不碰原件**：门闩在引导域手里，它直接授进 `target` 那张表，回一段"坐标 + 号"的记录；
//! 本域只做一次转投（一整段原样推过去）。

use alloc::string::String;
use alloc::string::ToString;
use env::{Pair, Wait};
use protocol::debug;

use env::{Access, Key, Mark, Policy};
use protocol::driver::hub::{ENROLL_MAX, Enroll};
use protocol::system::supply;
use protocol::system::supply::frame::Kind;
use protocol::system::supply::frame::{WANT_MAX, Want};

use crate::system::control::desk::Announce;

use super::{Control, Error, READY_MS, Service};
use crate::program::{Program, Setup};

impl Control {
    /// **登记一行**：只知道名字与它"怎么算起来"——此刻还没有身子（`spawn` 才挂）。
    ///
    /// **"怎么算起来"由 `setup` 推出**：有通道 ⇒ [`Announce::Channel`]（它起来时会交回
    /// 一枚孔，那枚到了才算起来）；否则 [`Announce::None`]（放行即起来）。这与旧装配表上那
    /// 两格（`announce` ＋ `channels`）**逐行等价**：有通道的那四台正是旧表里唯一写
    /// `Announce::Channel` 的四台。
    pub fn enlist(&mut self, program: &Program) -> Result<(), Error> {
        let name = program.name().to_string();
        self.table
            .register(name, announce_of(program.demand.setup))
            .map_err(|_| Error::Table)
    }

    /// **放行 + 入册**（装配那一相的后半）：**次序是硬的**——物料要落到它交回的那条路上，
    /// 故入册只能在放行之后（[`Control::start`] 之后才 [`Control::enroll`]）。
    ///
    /// **它不含"等就绪"那一手**（照实记：那一手从这一份里拆了出去）：等就绪要等的那几条通道里，
    /// `Machine` 那条"我起完了"要到**本域挂了树、也拿到物料之后**才铸得出来——而挂树由装配那一趟
    /// 在**放行之后**做。故两处的次序不同，各自把自己那一段读全：
    ///
    /// ```text
    ///   装配那一趟   放行 → 递物料 → 挂板 / 挂树 → **等就绪**
    ///   线上那条路   放行 → 递物料 →              **等就绪**（线上没有挂板 / 挂树那两手）
    /// ```
    ///
    /// 这也是"起一条"那条次序**只有两处**、且两处都把[放行 → 递料]这一段从这一手取的原因。
    pub fn launch(
        &mut self,
        program: &Program,
        name: String,
        service: &mut Service,
    ) -> Result<(), Error> {
        // **一、放行**（不等就绪）。
        self.start(name.as_str(), service)?;
        // **二、递物料**（只对声明了 `Setup::Machine` 的那一台；别的台这一步是空转）。
        // **等就绪**由调用方接在它该在的位置（见本手的注）。
        if let Some(load) = program
            .demand
            .setup
            .iter()
            .find(|s| s.machine())
            .map(Setup::channel)
        {
            self.enroll(name, service, load)?;
        }
        Ok(())
    }

    /// **入册**：把**这台机器的全部可领之物**交进收方（那一条声明了 `Setup::Machine` 的通道）。
    ///
    /// 前置：它**已经起来**（[`Control::start`] 之后）——通道那一头才认得上，记录也才落得进去。
    ///
    /// 三步：
    ///
    /// 1. **枚举全机**（[`crate::system::machine::Machine::devices`]）＋ 那两件按**已知坐标**的
    ///    （设备树本体 / 门铃——它们不在树里，没有"哪一类"可判）；
    /// 2. **逐段向引导域领**（`WANT_MAX` 一块）：坐标由本域翻（类 → 区那一条权威仍在读树的地方）；
    /// 3. **一整段推过去**（[`Enroll`]：条数 ＋ 那几条 `Pair` 记录，一个字节都不翻译）。
    ///
    /// **段尾那一条恒是设备树本体**（`wants[0]`）：收方要**先**把树读一遍，才知道哪一条记录是
    /// 哪一台（名 / 类 / 线）。次序即契约。
    ///
    /// **块内失败退成逐条**（照实记）：一块五条里有一条领不到（这台机器没有那一件 / 授不出），
    /// 整块会一起失败——而其余四条是好的。故那一块**逐条重来**：领得到的照收，领不到的打一行
    /// 读数、那一台就不在收方账上（缺一台不影响别的台）。
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
        if !link.claim(*task, Mark::of(load), Wait::AtMost(READY_MS)) {
            return Err(Error::Step("no channel"));
        }
        // **那头齐了没有**：写端在不在（原 `wire` 那一格）。
        let Some(tx) = link.tx() else {
            return Err(Error::Step("no channel"));
        };

        // 一、这一段要有哪几样。
        let devices = self
            .machine
            .devices()
            .ok_or(Error::Step("no room for devices"))?;
        let total = devices.len() + 2;
        if total > ENROLL_MAX {
            return Err(Error::Step("too many devices"));
        }
        let mut wants: alloc::vec::Vec<Want> = alloc::vec::Vec::new();
        wants
            .try_reserve(total)
            .map_err(|_| Error::Step("no room for devices"))?;
        // **形态照源枚**：设备那几段带 `ONLY`（内核就是那么发的：一枚门闩只许一个使用者），
        // 树与门铃不带。**这四条都带 `VEST`**：收方（设备账那一台）的全部工作就是**再授出**
        // （把每一台交到它认领者手里）——不带 `VEST` 它就一台都交不出去。
        //
        // 坐标那一格：树与门铃按**已知坐标**要（它们不在树里），设备按**区**要（那是内核造门闩
        // 的坐标）。
        wants.push(Want::new(
            Key::dtb(),
            Kind::Pole,
            Access::FETCH,
            Policy::VEST,
        ));
        wants.push(Want::new(
            Key::irq(),
            Kind::Nole,
            Access::FETCH,
            Policy::VEST,
        ));
        for device in &devices {
            wants.push(Want::new(
                device.key,
                Kind::Pole,
                Access::FETCH_STORE,
                Policy::VEST | Policy::ONLY,
            ));
        }

        // 二、逐段领（`WANT_MAX` 一块，块内失败退成逐条）。
        let mut records = [Pair::NONE; ENROLL_MAX];
        let mut got = 0usize;
        let mut at = 0usize;
        let mut reply = [0u8; supply::REPLY_CAP];
        while at < wants.len() {
            let end = core::cmp::min(at + WANT_MAX, wants.len());
            match supply::client::draw(
                &self.boot,
                *task,
                &wants[at..end],
                &mut reply,
                Wait::AtMost(READY_MS),
            ) {
                Ok(said) => {
                    for pair in said.records() {
                        records[got] = *pair;
                        got += 1;
                    }
                }
                // 这一块没成 ⇒ 逐条重来（好在:领得到的照收）。
                Err(_) => {
                    for want in &wants[at..end] {
                        let one = [*want];
                        match supply::client::draw(
                            &self.boot,
                            *task,
                            &one,
                            &mut reply,
                            Wait::AtMost(READY_MS),
                        ) {
                            Ok(said) => {
                                if let Some(pair) = said.records().first() {
                                    records[got] = *pair;
                                    got += 1;
                                }
                            }
                            // 领不到的那一台：**一行读数**，这一台不在册上。
                            Err(_) => debug!(
                                "system: enroll {} skipped {:#x}",
                                name.as_str(),
                                want.key().and_then(|key| key.base()).unwrap_or(0)
                            ),
                        }
                    }
                }
            }
            at = end;
        }

        // 三、一整段推过去。
        let Some(enroll) = Enroll::of(&records[..got]) else {
            return Err(Error::Step("too many devices"));
        };
        protocol::communication::sender::Sender::<Enroll>::from_token(tx)
            .send(enroll)
            .map_err(|_| Error::Step("no channel"))?;
        debug!("system: enrolled {} supplies for {}", got, name.as_str());
        Ok(())
    }
}

// **照实记（`pairs_of` 那一手退场了）**：它从前把"回单里那段裸字节"按 `PAIR_LEN` 步长解成
// `Pair`（解不动的跳过）；`draw` 改成返 `Reply`（解码本来就已经把那几条收进表里了）之后，
// 那一段裸字节在调用点已经不存在 ⇒ 那一手与它那条"跳过"的判据一并退场。

/// **怎么算"它起来了"**：由这一行的 `setup` 推出（见 [`Control::enlist`]）。
fn announce_of(setup: &[Setup]) -> Announce {
    if setup.is_empty() {
        Announce::None
    } else {
        Announce::Channel
    }
}

/// **装通道**（"配"那一相）：按这一台 `setup` 里那几格逐条装上——**记号 = 通道名**，
/// 放行后按同一个记号逐条认领（[`service::ready`](super::service::ready)）。
///
/// **自由函数**：它只碰通道，不碰 `Control` 的任何一格（与 [`connect`](super::connect) 那一手
/// 同一句正文——"只碰通道"的那一层做成方法就是白加的壳）。两处叫它：装配那一趟
/// （[`crate::system::Assembly::assemble`]）与线上那条 [`Control::release`]。
pub fn connect_all(program: &Program, service: &mut Service) -> Result<(), Error> {
    for s in program.demand.setup {
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
