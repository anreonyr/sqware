//! System 里那一份**设备 DMA 管理者**：它把一台真实 PCI 端点的每一页工作内存
//! 逐一保留、发布，再把那一段精确 BAR 经供给窗口那一层交给一台**跨 Team** 的端点
//! 服务，并在服务死后由 System 自己收口。
//!
//! 角色的界线与 `harness/probe/iommu/dma` 同一套，但这里是产品那一侧：
//! - **内存那一半**（DDT / DC / Sv39 表 / 三类队列 / 描述符 / 数据）与 IOMMU 寄存器
//!   归 System，那是它按保留发布、并按 `Platform` 独立停住的那一份；
//! - **设备那一半**（配置空间之后的寄存器面）归端点服务，它手里只有一枚精确到一根
//!   BAR 的窗口与几个 IOVA——它自己核 `Open` 出来的长度就是那一段，不多一页；
//! - 完成记录**不搬给服务的裸 VA**：服务只负责把设备立起来、打一次铃；
//!   设备写回的 `used` 环在 System 保留的页里，由 System 自己看。
//!
//! 换页/寄存器右键仍是 `crate::driver::iommu` 与 `crate::driver::pci` 那两套机制。
//!
//! **一帧一格，不阻塞**：起手那一串要等硬件的步骤（抬两台队列、`IOFENCE.C` 完成、
//! `DDTP` 回读）每帧只推一格，期限随那一格保留（[`regs::Handshake`]）；没等到就把
//! 引用原样留着，下一帧接着来。System 的帧因此从不因等待而停（见
//! [`crate::system::app::device::begin`]）。

use alloc::boxed::Box;

use env::{AcquireSource, Call, Mark, Name, PAGE_SIZE, Page, Permission, PieToken, TaskId, Wait};
use system_api::supply::LeaseId;

use resource::dock::Dock;
use resource::raw::Hole;

use crate::driver::iommu::{
    context::DeviceContext,
    fence::Fence,
    handshake::Handshake,
    order,
    pages::Pages,
    queue::{self, CommandQueue},
    regs, table,
};
use crate::driver::pci::ecam::{self, Ecam};
use crate::driver::pci::endpoint::Endpoint;
use crate::driver::pci::virtio::{self, Caps};
use crate::system::app::supplies::Supplies;
use crate::system::device::endpoint_drain::EndpointDrain;
use crate::system::device::fault::FaultOnce;
use crate::system::device::path::Path;
use crate::system::device::platform::IommuPlatform;
use crate::system::device::stage::Stage;
use crate::system::device::start::Start;
use crate::system::device::started::Started;
use crate::system::loader::{Image, Loader};
use crate::system::supply::retention::{Close, DeviceRange, Failure, Platform, Scope};
use crate::system::supply::{Origin, PrivateRoot, RootAssignment, RootRequest};

/// 设备树里 `riscv,iommu` 那段 `reg` 的起址。
const IOMMU_BASE: u64 = 0x301_0000;
/// 设备树里 `pci@…` 的 `reg`：ECAM 那一段的起址。
const ECAM_BASE: u64 = 0x3000_0000;
/// 设备树 `ranges` 里的 64 位 PCI 窗口：精确 BAR 就从这段里取。
const APERTURE_BASE: u64 = 0x4_0000_0000;
const APERTURE_SIZE: u64 = 0x4_0000_0000;
/// 端点：virtio 的供应商号与块设备号（过渡态 / 纯 1.x 两个号都收）。
const VENDOR: u16 = 0x1AF4;
const BLOCK: [u16; 2] = [0x1001, 0x1042];
/// 交给服务的那一段 BAR 与交给服务那一枚坐标孔的门牌。
const BAR_MARK: Mark = Mark::of("system-dma-bar");
const COORD_MARK: Mark = Mark::of("system-dma-coord");
/// 管理者自己那一枚：读写、可再授出，形态位照源那一枚（`ONLY` 必须一致）。
const OWNER_SUBSET: Permission = Permission::FETCH
    .union(Permission::STORE)
    .union(Permission::VEST)
    .union(Permission::ONLY);
/// 交给服务那一枚：只能读写，**不能再授出**。
const DRIVER_SUBSET: Permission = Permission::FETCH
    .union(Permission::STORE)
    .union(Permission::ONLY);
/// 保留窗里的那些页。
const DDT: usize = 0;
const ROOT: usize = 1;
const MIDDLE: usize = 2;
const LEAF: usize = 3;
const COMMANDS: usize = 4;
const FAULTS: usize = 5;
const FENCE: usize = 6;
const DESC: usize = 7;
const AVAIL: usize = 8;
const USED: usize = 9;
const DATA: usize = 10;
const HEADER: usize = 11;
const STATUS: usize = 12;
/// 窗里的页数：`DDT..=STATUS`。
const COUNT: usize = STATUS + 1;
/// 布给设备的那一段（`DESC..=STATUS` 六页）。
const DEVICE_PAGES: usize = STATUS - DESC + 1;
/// 设备看到的那一段 IOVA：与真实物理页不同，故“翻过去了”不是零距离。
///
/// 一个进程只跑一轮：那一段精确 BAR 窗口是从 aperture 上**唯一一次**交付出来的
/// （`ONLY` 的分支一交出去，源就不能再派生第二枚），故两轮分属两个景、两个进程。
const IOVA_BASE: usize = 0x2_0000_0000;
/// 一轮读一个扇区；头是设备只读的那 16 字节。
const SECTOR: usize = 512;
const HEADER_BYTES: usize = 16;
/// 硬件写回的可写字节数：数据 ＋ 状态（只读的头不算）。
const WRITTEN_BYTES: u32 = (SECTOR + 1) as u32;
/// 块请求类型与状态。
const BLK_IN: u32 = 0;
const BLK_OK: u8 = 0;
/// 描述符标志：`NEXT` / `WRITE`（设备侧可写）。
const DESC_NEXT: u16 = 1;
const DESC_WRITE: u16 = 2;
/// 负证那一轮先铺在数据页里的哨兵。
const SENTINEL: u8 = 0x5A;
/// 一轮的起手：先摆好第一轮那一段请求。
const ROUND_FIRST: u16 = 0;
/// 交给服务的那一帧坐标的字数。
const COORD_WORDS: usize = 9;
/// 确认之后等设备再推一次的帧数（System 的一帧一帧，不阻塞）。
const PROVE_FRAMES: usize = 8;

/// 产品 DMA 管理者：一次只跑一台端点。
pub struct Dma {
    /// 起手走到哪一步（见 [`Start`]）。
    start: Start,
    iommu: Option<Dock>,
    ecam: Option<Dock>,
    /// System 自己那一枚 BAR 窗口：收口时由它打响那一次门铃（不经过任何服务）。
    bar: Option<Dock>,
    retention: PieToken,
    aperture: PieToken,
    base: usize,
    range: Option<DeviceRange>,
    /// 这一范围是否真的在 `Holds` 上登记过（`close` 只对登记过的认账）。
    attached: bool,
    /// 发布门是否已经点上——没点上 = 设备从来没可达过。
    published: bool,
    /// 那一份登记在 `Holds` 上的编号（发布门的成败都记在它那一格）。
    retained: Option<usize>,
    /// 收子任务那两条命令已经发出去了（每帧 `join` 探它落地，不重发）。
    reclaim_sent: bool,
    pages: Option<Pages>,
    /// 武装那一串用的队列（`setup` 里建好，一帧推一格）。
    queues: Option<CommandQueue>,
    /// 当前那一格等硬件的期限（跨帧保留）。
    handshake: Option<Handshake>,
    caps: Option<Caps>,
    /// 已经配置好的那一台端点。
    endpoint: Option<Endpoint>,
    service: Option<TaskId>,
    /// 服务那一 Team（收工时按 Team 收，与 `ruin` 同一条路）。
    team: Option<env::TeamId>,
    /// 管理者自己那一枚精确 BAR 窗口（交付时从它派生给服务那一枚）。
    window: PieToken,
    /// 那一窗在 aperture 里从第几页起、共几页。
    first: usize,
    pages_in_bar: usize,
    /// 第一轮已完成（设备真的写回了 `used` 环）。
    served: bool,
    /// 已经请求收口（正常关闭或收服务）；不重发。
    closing: bool,
    /// 确认失败那一段已经报过（注入景里那一次 `Pending`）。
    pending_seen: bool,
    /// 确认之后那一次“自己打响门铃”已经做过。
    proved: bool,
    /// 等设备再推一次的剩几帧。
    prove_left: usize,
    /// 打铃那一时刻的 `used.idx`。
    prove_before: u16,
    reported: bool,
    /// 这一景走哪一条路（见 [`Path`]）。
    path: Path,
    /// 服务那一台镜像（装配期就从清单里取好，运行期不重查）。
    image: Option<&'static [u8]>,
    kind: Option<env::ProgramKind>,
    /// 确认后验也报过了。
    done: bool,
}

impl Dma {
    /// `image` / `kind` 是那一台端点服务在镜像里的字节与特权级；`path` 由这一景的装配给出
    /// （普通产品为 [`Path::Normal`]）。
    pub const fn new(
        image: Option<&'static [u8]>,
        kind: Option<env::ProgramKind>,
        path: Path,
    ) -> Self {
        Self {
            start: Start::Fresh,
            iommu: None,
            ecam: None,
            bar: None,
            retention: PieToken::NONE,
            aperture: PieToken::NONE,
            base: 0,
            range: None,
            attached: false,
            published: false,
            retained: None,
            reclaim_sent: false,
            pages: None,
            queues: None,
            handshake: None,
            caps: None,
            endpoint: None,
            service: None,
            team: None,
            window: PieToken::NONE,
            first: 0,
            pages_in_bar: 0,
            served: false,
            closing: false,
            pending_seen: false,
            proved: false,
            prove_left: 0,
            prove_before: 0,
            reported: false,
            path,
            image,
            kind,
            done: false,
        }
    }

    pub fn running(&self) -> bool {
        matches!(self.start, Start::Running)
    }

    /// 发现 → 保留 → 发布 → 交付 → 放开服务，**一帧推一步**。
    ///
    /// 不在任何一步阻塞等设备：要等硬件的那几步（抬队列、`IOFENCE` 完成、`DDTP` 回读）
    /// 本帧只做一次写入/一次回读，没等到就返 [`Started::Starting`]，下一帧接着推。
    /// 起手失败一次就**封口不再重试**，但把已经拿到的东西**主动**交回去
    /// （见 [`Dma::rollback`]）——不指望一个还没放开的任务自己死。
    pub fn begin(
        &mut self,
        loader: &mut Loader,
        supplies: &mut Supplies,
    ) -> Result<Started, Failure> {
        if matches!(self.start, Start::Failed) {
            // 已经封口：只把没收干净的那点收干净（`doom`/`slay` 幂等到成功）。
            self.rollback(supplies);
            return Ok(Started::Absent);
        }
        if self.running() {
            return Ok(Started::Running);
        }
        loop {
            match self.advance(loader, supplies) {
                Ok(Stage::Continue) => continue,
                Ok(Stage::Waiting) => return Ok(Started::Starting),
                Ok(Stage::Done(started)) => return Ok(started),
                Err(failure) => {
                    self.start = Start::Failed;
                    // 发布门的成败记在它自己那一格（发布之后失败先记上，再回退）。
                    if let Some(id) = self.retained.filter(|_| self.published) {
                        let _ = supplies.retention.publish_end(id, Err(failure));
                    }
                    self.rollback(supplies);
                    return Err(failure);
                }
            }
        }
    }

    /// 起手失败的回退：**主动**把子任务收进既有死亡路径、把范围关成 `Fault`。
    ///
    /// 收不干净就把手柄**留着**（不 `take` 掉），下一帧接着收：`doom`/`slay` 的真实结果
    /// 要报出来，不能用 `let _` 吐成“已清”而无凭据。
    ///
    /// 范围：登记过（`attached`）才关得了，没 attach 过时不假装关过（`close` 只会答
    /// `Unknown`，那一格不能按“已关”打印）；`close` 的结果也按真实报。关成 `Fault` 之后
    /// `Holds` 会把它推到 `Stopped` 并把登记释放，故不会有一个空转在 `Open` 的范围。
    ///
    /// 本地所有者：**发布之前**失败 = 设备从来没可达过，本域那几段页归 `settle` 直接
    /// 结清；**发布之后**失败 = 停机凭据归平台，`pages` 也要留到 `settle` 确认之后才还。
    /// 两档都归 `settle`——那里才是“最后一次读 MMIO”之后（见 [`Dma::finish`]）。
    fn rollback(&mut self, supplies: &mut Supplies) {
        self.reclaim();
        if let Some(range) = self.range {
            if !self.attached {
                self.say("system-dma: startup failed, no registered scope");
            } else if !self.closing {
                self.closing = true;
                let closed = supplies.retention.close(range, Close::Fault);
                self.say(&alloc::format!(
                    "system-dma: startup failed, range close={closed:?}"
                ));
            }
        }
    }

    /// 收子任务：`doom` ＋ `slay_team` **只发一次**，两个结果原样报出来；之后每帧
    /// `join` 一次探它**真的没了**。
    ///
    /// 「没有留着活僵尸」要的是最后那一格（`join` 看到已回收），不是“命令发出去了”
    /// ——所以收干净之前 `service`/`team` 两个手柄都**不** `take` 掉，下一帧接着收。
    /// 命令真失败时失败写在 `child doomed` 那一行里，而 `reclaimed` 那一行就不会出现：
    /// 只报成功、或沉默地把失败吐掉，两种都做不到。
    fn reclaim(&mut self) {
        let Some(service) = self.service else {
            return;
        };
        if !self.reclaim_sent {
            self.reclaim_sent = true;
            let doom = env::room::doom(service).map_err(|error| error.source);
            let slay = self
                .team
                .map(|team| env::unit::slay_team(team).map_err(|error| error.source));
            self.say(&alloc::format!(
                "system-dma: startup failed, child doomed doom={doom:?} slay={slay:?}"
            ));
        }
        if matches!(env::unit::join_task(service, Wait::POLL), Ok(true)) {
            self.service = None;
            self.team = None;
            self.say("system-dma: startup failed, child reclaimed");
        }
    }

    /// 本域对设备那几段图的**最后一次访问**已经过完：显式把寄存器 / ECAM / BAR 三段图关掉，
    /// 并把 `range` 与 `endpoint` 放掉——之后 `settle` 再进来就不会再读 MMIO（`range` 为 `None` 直接 `Done`）。
    ///
    /// 现在才关是因为平台对象只持本域地址（`usize`），而 `settle` 的停机现场要从这三段读；
    /// 图一关再读就是悬空。
    fn finish(&mut self) {
        self.range = None;
        self.endpoint = None;
        if let Some(dock) = self.iommu.take() {
            let _ = dock.shut();
        }
        if let Some(dock) = self.ecam.take() {
            let _ = dock.shut();
        }
        if let Some(dock) = self.bar.take() {
            let _ = dock.shut();
        }
    }

    /// 一帧推一步：装配 → 武装 → 交付。
    fn advance(&mut self, loader: &mut Loader, supplies: &mut Supplies) -> Result<Stage, Failure> {
        match self.start {
            Start::Fresh => {
                self.setup(loader, supplies)?;
                Ok(Stage::Continue)
            }
            Start::Faults | Start::Commands | Start::Fence | Start::Ddtp | Start::Master => {
                self.arm()
            }
            Start::Armed => {
                // 受控注入（只这一景）：发布与武装都过了、交付之前停一次。
                if self.path == Path::StartupAfterPublish {
                    self.say("system-dma: startup fault injected after publish");
                    return Err(Failure::Unavailable);
                }
                self.deliver(supplies)?;
                self.start = Start::Running;
                Ok(Stage::Done(Started::Running))
            }
            Start::Running => Ok(Stage::Done(Started::Running)),
            Start::Absent | Start::Failed => Ok(Stage::Done(Started::Absent)),
        }
    }

    /// 装配：开图、发现端点、定范围、立服务、取窗口与页、装平台、挂登记，
    /// 并**点上发布标记**、铺好表与描述符。要等硬件的那一串留给 [`Dma::arm`]。
    fn setup(&mut self, loader: &mut Loader, supplies: &mut Supplies) -> Result<(), Failure> {
        let Some(iommu) = supplies
            .accounts
            .token(Name::Page(Page::Region(IOMMU_BASE)))
        else {
            self.start = Start::Absent;
            return Ok(());
        };
        // 两轮共用同一片寄存器窗与 ECAM：只开一次图（第一轮那一开）。
        if self.iommu.is_none() {
            let Some(ecam) = supplies.accounts.token(Name::Page(Page::Region(ECAM_BASE))) else {
                self.start = Start::Absent;
                return Ok(());
            };
            let Some(aperture) = supplies
                .accounts
                .token(Name::Page(Page::Region(APERTURE_BASE)))
            else {
                self.start = Start::Absent;
                return Ok(());
            };
            self.retention = supplies
                .accounts
                .token(Name::Call(Call::Retain))
                .ok_or(Failure::Denied)?;
            self.aperture = aperture;
            let iommu_dock = Dock::open(iommu).map_err(|_| Failure::Unavailable)?;
            self.base = iommu_dock.view().base();
            self.iommu = Some(iommu_dock);
            self.ecam = Some(Dock::open(ecam).map_err(|_| Failure::Unavailable)?);
        }
        let ecam_base = self
            .ecam
            .as_ref()
            .map(|dock| dock.view().base())
            .ok_or(Failure::Unavailable)?;

        // 发现端点：能力表说四个区在哪一根 BAR，端点把那一段落到 aperture 窗口里。
        let requester = ecam::find(ecam_base, VENDOR, &BLOCK).ok_or(Failure::Unavailable)?;
        let scan = Ecam::new(ecam_base, requester);
        let (bar_index, caps) = Caps::read(&scan).map_err(|_| Failure::Unavailable)?;
        let endpoint =
            Endpoint::configure(&scan, bar_index, (APERTURE_BASE, APERTURE_SIZE)).map_err(fail)?;
        // 落位之后立刻回读：`configure` 不动 bus-master，此刻必须还是 0。
        let master = endpoint.master();
        self.say(&alloc::format!(
            "system-dma: endpoint configured requester={} bar={:#x} master={master:#x}",
            endpoint.requester(),
            endpoint.address(),
        ));
        self.endpoint = Some(endpoint);
        self.caps = Some(caps);
        let first = ((endpoint.address() - APERTURE_BASE) / PAGE_SIZE as u64) as usize;
        let pages_in_bar = (endpoint.size() as usize).div_ceil(PAGE_SIZE);
        // 这一轮的范围是**设备能经 IOMMU 到的那一段内存**（不是 BAR 那一段 MMIO）：
        // 要停住的“外部访问”正是它。
        let base = IOVA_BASE;
        let range = DeviceRange::new(base as u64, (DEVICE_PAGES * PAGE_SIZE) as u64)
            .ok_or(Failure::Denied)?;
        // 范围一旦定下就记在管理者身上：后面任何一步失败都能把它关成 `Fault`。
        self.range = Some(range);

        // 先把那台服务任务立起来（**按住**：坐标没铺好之前不放它跑）。
        let (Some(image), Some(kind)) = (self.image, self.kind) else {
            return Err(Failure::Unavailable);
        };
        let unit = loader
            .build(Image { bytes: image, kind })
            .map_err(|_| Failure::Pending)?;
        let team = unit.team();
        let service = unit.spawn(&[], 0).map_err(|_| Failure::Pending)?;
        // 子任务一建出来就归管理者：起手后面任何一步失败都**主动**收它。
        self.team = Some(team);
        self.service = Some(service);

        // 两枚窗口：管理者自己那一枚（收口时打响那一次门铃、端点复位都用它），以及
        // 从它收窄出来、经产品供给窗口那一层交给服务的那一枚（`NativePieEffects`）。
        //
        // **次第一：先把本域那一段图拿下来，再把那一枚交出去**——`ONLY` 的成功
        // 派生是**移交**：交出去之后发送方那枚就不能再 `Open` 了，故图必须在前面。
        // 又：`ONLY` 的分支一交出去，源就不能再派生第二枚，故一个进程只有这一轮。
        let mine = env::pie::accord_window(
            self.aperture,
            env::unit::self_id(),
            OWNER_SUBSET,
            BAR_MARK,
            first,
            pages_in_bar,
        )
        .map_err(|_| Failure::Full)?;
        let bar = Dock::open(mine).map_err(|error| {
            crate::debug::put(&alloc::format!(
                "system-dma: bar dock open {:?}",
                error.source
            ));
            Failure::Closed
        })?;
        self.bar = Some(bar);
        let bar_base = self
            .bar
            .as_ref()
            .map(|dock| dock.view().base())
            .ok_or(Failure::Closed)?;

        // 取这段工作内存：本域一份（建表/读完成），独立保留那一份在 `retain_memory`。
        let pages = Pages::take(self.retention, COUNT).map_err(fail)?;
        self.pages = Some(pages);
        let pages = self.pages.as_ref().ok_or(Failure::Unknown)?;
        let queue = CommandQueue::new(self.base, pages, COMMANDS, FAULTS, FENCE);
        let inner = IommuPlatform::new(
            range,
            queue,
            Some(EndpointDrain {
                endpoint,
                bar: bar_base,
                caps,
            }),
        );
        // 只故障景：装一层“第一次停住答 `Pending`”，验“确认失败仍保留”。
        let platform: Box<dyn Platform> = if self.path == Path::Fault {
            Box::new(FaultOnce::new(inner))
        } else {
            Box::new(inner)
        };
        supplies
            .retention
            .attach(Scope::new(range, service, Some(platform)))?;
        self.attached = true;
        let id = supplies.retain_memory(
            range,
            AcquireSource::Source {
                token: pages.source(),
                offset: 0,
                count: COUNT,
            },
        )?;
        self.retained = Some(id);
        // 受控注入（只这一景）：`spawn` 之后、发布之前停一次——范围已登记、可达标记
        // 还没点。它验的是“部分发布的失败真的被隔断”，不是“硬件超时”。
        if self.path == Path::StartupBeforePublish {
            self.say("system-dma: startup fault injected before publish");
            return Err(Failure::Unavailable);
        }
        // 发布门：**先把可达标记点上**（执行发布之前），再铺表、描述符。
        supplies.retention.publish_begin(id)?;
        self.published = true;
        let context = DeviceContext::first_stage(pages.physical(ROOT));
        let requester = endpoint.requester() as usize;
        for (index, word) in context.words().iter().enumerate() {
            pages.put64(DDT, requester * 64 + index * 8, *word);
        }
        table::map_pages(pages, ROOT, MIDDLE, LEAF, base, DESC, DEVICE_PAGES)
            .map_err(|_| Failure::Denied)?;
        arm(pages, base, ROUND_FIRST);

        // 武装那一串（立队列 → 失效 → `IOFENCE` 完成 → `DDTP`）归 `arm`，一帧一格。
        self.queues = Some(CommandQueue::new(self.base, pages, COMMANDS, FAULTS, FENCE));
        self.window = mine;
        self.first = first;
        self.pages_in_bar = pages_in_bar;
        self.start = Start::Faults;
        Ok(())
    }

    /// 武装：每帧推一格。
    fn arm(&mut self) -> Result<Stage, Failure> {
        match self.start {
            Start::Faults => {
                if !self.raised(regs::FQCSR)? {
                    return Ok(Stage::Waiting);
                }
                self.start = Start::Commands;
                Ok(Stage::Continue)
            }
            Start::Commands => {
                if !self.raised(regs::CQCSR)? {
                    return Ok(Stage::Waiting);
                }
                self.queues
                    .as_mut()
                    .ok_or(Failure::Unknown)?
                    .invalidate_all();
                self.start = Start::Fence;
                Ok(Stage::Continue)
            }
            Start::Fence => match self.queues.as_mut().ok_or(Failure::Unknown)?.fence_step() {
                Fence::Settled => {
                    self.start = Start::Ddtp;
                    Ok(Stage::Continue)
                }
                Fence::Waiting => Ok(Stage::Waiting),
                Fence::Expired => Err(Failure::Unavailable),
            },
            Start::Ddtp => {
                let ddt = self.pages.as_ref().ok_or(Failure::Unknown)?.physical(DDT);
                if !self.programmed(regs::DDTP_MODE_1LVL, ddt)? {
                    return Ok(Stage::Waiting);
                }
                self.start = Start::Master;
                Ok(Stage::Continue)
            }
            Start::Master => {
                let endpoint = self.endpoint.ok_or(Failure::Unknown)?;
                let master = endpoint.grant_master();
                let granted = master == ecam::COMMAND_MASTER;
                self.say(&alloc::format!(
                    "system-dma: endpoint bus-master={master:#x} granted={granted}"
                ));
                if !granted {
                    return Err(Failure::Unavailable);
                }
                self.start = Start::Armed;
                Ok(Stage::Continue)
            }
            _ => Ok(Stage::Continue),
        }
    }

    /// 抬一台队列这一格：写没写过就写一次（起一次握手），然后**单次读**。
    ///
    /// `busy` 未落且在期限内 → `Ok(false)`（下一帧接着读）；落了但 `ON` 没立起来
    /// 或带故障位 → 硬失败；写只做一次，期限随那一格保留。
    fn raised(&mut self, csr: usize) -> Result<bool, Failure> {
        if self.handshake.is_none() {
            let handshake = self.queues.as_ref().ok_or(Failure::Unknown)?.raise(csr);
            self.handshake = Some(handshake);
        }
        let seen = regs::read32(self.base, csr);
        if seen & regs::QUEUE_BUSY != 0 {
            return match self.handshake.is_some_and(|handshake| handshake.expired()) {
                true => Err(Failure::Unavailable),
                false => Ok(false),
            };
        }
        if !queue::active(seen, regs::QUEUE_ON) || seen & regs::QUEUE_MEM_FAULT != 0 {
            return Err(Failure::Unavailable);
        }
        self.handshake = None;
        Ok(true)
    }

    /// `DDTP` 那一格：写没写过就写一次，然后**单次读**。
    fn programmed(&mut self, mode: u64, root: usize) -> Result<bool, Failure> {
        if self.handshake.is_none() {
            // 表与队列都布好、`IOFENCE` 拿到完成记录之后才接上设备目录表。
            order::sync();
            let handshake =
                regs::ddtp_start(self.base, mode, Some(root)).map_err(|_| Failure::Unavailable)?;
            self.handshake = Some(handshake);
        }
        match regs::ddtp_poll(self.base, mode) {
            Ok(_) => {
                self.handshake = None;
                Ok(true)
            }
            Err((_, seen)) if seen & regs::DDTP_BUSY != 0 => {
                match self.handshake.is_some_and(|handshake| handshake.expired()) {
                    true => Err(Failure::Unavailable),
                    false => Ok(false),
                }
            }
            Err(_) => Err(Failure::Unavailable),
        }
    }

    /// 交付：把那一枚交出去、把坐标铺过去、放开服务。
    fn deliver(&mut self, _supplies: &mut Supplies) -> Result<(), Failure> {
        let service = self.service.ok_or(Failure::Pending)?;
        // 把那一枚交出去（`ONLY`：发送方那一枚就此不可再 `Open`，图已在前面拿下）。
        let root_window = PrivateRoot::derive(
            RootAssignment {
                shared: self.window,
                index: 0,
                window: Some((self.first, self.pages_in_bar)),
            },
            RootRequest {
                id: LeaseId(1),
                origin: Origin {
                    requester: env::unit::self_id(),
                    authority: self.window.get(),
                },
                owner: service,
                permission: DRIVER_SUBSET,
            },
        )
        .map_err(|_| Failure::Denied)?;
        let seed = root_window.token();
        let requester = self.endpoint.as_ref().ok_or(Failure::Unknown)?.requester();
        let bar_base = self
            .bar
            .as_ref()
            .map(|dock| dock.view().base())
            .unwrap_or(0);
        crate::debug::put(&alloc::format!(
            "system-dma: endpoint requester={} bar={bar_base:#x} window={}+{} seed={}",
            requester,
            self.first,
            self.pages_in_bar,
            seed.get(),
        ));

        // 坐标只经那一枚孔过去：种子、页数、能力表四格、三段 IOVA。
        // 服务还按着（没 embark），故是一个**非阻塞的一次尝试**：装不下就当场失败
        // 回退，不在 System 的一帧里 park。
        let (common, notify, multiplier, device) = self.caps.ok_or(Failure::Unavailable)?.parts();
        let base = IOVA_BASE;
        let writer =
            ipc::session::establish::give(service, COORD_MARK).map_err(|_| Failure::Conflict)?;
        let frame = [
            seed.get(),
            self.pages_in_bar,
            common as usize,
            notify as usize,
            multiplier as usize,
            device as usize,
            iova(base, DESC),
            iova(base, AVAIL),
            iova(base, USED),
        ];
        let mut bytes = [0u8; COORD_WORDS * 8];
        for (at, word) in frame.iter().enumerate() {
            bytes[at * 8..at * 8 + 8].copy_from_slice(&word.to_le_bytes());
        }
        Hole::from_raw(writer)
            .push(&bytes, Wait::POLL)
            .map_err(|_| Failure::Pending)?;

        env::unit::embark_task(service).map_err(|_| Failure::Pending)?;
        Ok(())
    }

    /// 收口：看到设备写回 → 请求关闸（正常景 `Close::Normal`，故障景收服务走
    /// `offline(Fault)`）→ 等生命周期**确认并释放**之后才算收完。
    ///
    /// 它**永远返 `Done`**：关闸与释放的节奏归既有生命周期（`retention.sweep`），
    /// 这里只是看结果；返 `Pending` 会把这帧后面的步骤挡住，反而让 `sweep` 跑不到。
    /// 但“还在保留”与“已释放确认”是**两个不同状态**，各报各的，不统一算绿。
    pub fn settle(&mut self, supplies: &mut Supplies) -> ::schedule::Progress {
        use ::schedule::Progress;
        let Some(range) = self.range else {
            return Progress::Done;
        };
        // 完成记录：读一轮，报一次正证。
        if let Some(pages) = self.pages.as_ref() {
            let (used, len, status) = completion(pages);
            if !self.served && used >= 1 {
                self.served = true;
                let sector_ok = sector_matches(pages);
                // 正证两个数：可写两段的和（只读的头不算）与该写回的状态。
                let written_ok = len == written_bytes() && status == block_ok();
                self.say(&alloc::format!(
                    "system-dma: served used={used} len={len} status={status} written_ok={written_ok} sector_ok={sector_ok}"
                ));
            }
        }
        // 请求关闸：正常景显式正常关闭；故障景收服务（它的死走 `offline` → `Fault`）。
        if self.served && !self.closing {
            self.closing = true;
            // 关闸**之前**的现场：这几格此刻还不是停的样子（目录表还接着、队列还开着），
            // 故它与释放后那一行对照起来才看得出判据不是白给的真。
            self.say(&alloc::format!("system-dma: at close {}", self.stopped()));
            if self.path != Path::Fault {
                let _ = supplies.retention.close(range, Close::Normal);
                self.say("system-dma: requested Close::Normal");
            } else if let Some(service) = self.service {
                let doom = env::room::doom(service).map_err(|error| error.source);
                let slay = self
                    .team
                    .map(|team| env::unit::slay_team(team).map_err(|error| error.source));
                self.say(&alloc::format!(
                    "system-dma: service doomed doom={doom:?} slay={slay:?}"
                ));
            }
        }
        // 确认没过的现场：保留、报一次、**不**往下走也不重启业务。
        let held = supplies
            .retention
            .occupied()
            .find(|(held, ..)| *held == range);
        if let Some((_, _, phase, count, failure)) = held {
            if failure.is_some() && !self.pending_seen {
                self.pending_seen = true;
                self.say(&alloc::format!(
                    "system-dma: NOT released holds={count} phase={phase:?} failure={failure:?}"
                ));
            }
            return Progress::Done;
        }
        // 到这里保留已放下 = 平台确认过了。**独立核一遍现场**（映射与保留在这一刻
        // 都还活着）：目录表 Off、两台队列的 `on` 落了、端点 bus-master 清了、设备
        // 状态与队列使能都回 0。任一没到位就 `stopped=false`——把 stop 换成空实现，
        // 这一行必红（这是“确认并释放”那一格的承重判据）。
        // 起手失败那一档：没有正证、也没有“确认后放页”那一格，只报现场。
        // （子任务已由 `rollback` 主动收进死亡路径，范围也已关成 `Fault`。）
        if !self.served {
            if !self.reported {
                self.reported = true;
                self.say(&alloc::format!(
                    "system-dma: startup failure settled {}",
                    self.stopped()
                ));
            }
            if let Some(pages) = self.pages.take() {
                let _ = pages.close();
            }
            self.finish();
            return Progress::Done;
        }
        if !self.reported {
            self.reported = true;
            self.say(&alloc::format!(
                "system-dma: confirmed and released {}",
                self.stopped()
            ));
        }
        // 再打一次铃并观察。**这一步只说明被复位过的端点是闲的**（设备模型已清掉
        // 它的 vring，门铃根本不进设备），**不是**“IOMMU 单独挡住一次在途设备”的负证——
        // 那条负证在 `--scene iommu-dma` 的探针里（同一 requester 再翻一次必须被拒，
        // 且故障队列记下 `DMA_DISABLED`）。
        // 确认之后由管理者自己那一枚窗口再打一次铃，验设备再也推不动。
        if !self.proved {
            self.proved = true;
            if let Some(pages) = self.pages.as_ref() {
                self.prove_before = pages.get16(USED, 2);
                sentinel(pages, DATA);
                pages.put8(STATUS, 0, 0xFF);
                arm(pages, IOVA_BASE, ROUND_FIRST + 1);
            }
            if let (Some(bar), Some(caps)) = (self.bar.as_ref(), self.caps) {
                ring_own(bar, &caps);
            }
            self.prove_left = PROVE_FRAMES;
            return Progress::Done;
        }
        if self.prove_left > 0 {
            self.prove_left -= 1;
            return Progress::Done;
        }
        if !self.done {
            self.done = true;
            let moved = self
                .pages
                .as_ref()
                .is_some_and(|pages| pages.get16(USED, 2) != self.prove_before);
            let held = self
                .pages
                .as_ref()
                .is_some_and(|pages| holds_byte(pages, DATA, SENTINEL));
            let status = self
                .pages
                .as_ref()
                .map(|pages| pages.get8(STATUS, 0))
                .unwrap_or(0xFF);
            self.say(&alloc::format!(
                "system-dma: reset endpoint idle used_moved={moved} sentinel_held={held} status_byte={status} requester={}",
                self.endpoint.map_or(u16::MAX, |endpoint| endpoint.requester()),
            ));
            // 到这里本域对那一段工作内存的**最后一次访问**已经过完：图与匿名来源
            // 可以收 — 归属侧是 System 自己，没有别人的窗口从这里派生过。
            if let Some(pages) = self.pages.take() {
                let _ = pages.close();
            }
            self.finish();
        }
        Progress::Done
    }

    fn say(&self, message: &str) {
        crate::debug::put(message);
    }

    /// 停止现场的独立读数（映射与保留在这一刻都还活着）：目录表 Off 且不忙、
    /// 两台队列的 `on` 落了、端点 bus-master 清了、设备状态与队列使能都回 0。
    fn stopped(&self) -> alloc::string::String {
        let ddtp = regs::read64(self.base, regs::DDTP);
        let cq = regs::read32(self.base, regs::CQCSR);
        let fq = regs::read32(self.base, regs::FQCSR);
        let master = match (self.ecam.as_ref(), self.endpoint.as_ref()) {
            (Some(_), Some(endpoint)) => endpoint.master(),
            _ => ecam::COMMAND_MASTER,
        };
        let (status, enable) = match (self.bar.as_ref(), self.caps) {
            (Some(bar), Some(caps)) => (
                virtio::status(bar.view().base(), &caps),
                virtio::queue_enable(bar.view().base(), &caps),
            ),
            _ => (0xFF, 0xFFFF),
        };
        let off = ddtp & 0xF == regs::DDTP_MODE_OFF && ddtp & regs::DDTP_BUSY == 0;
        let queues_off = cq & regs::QUEUE_ON == 0 && fq & regs::QUEUE_ON == 0;
        let stopped = off && queues_off && master == 0 && status == 0 && enable == 0;
        alloc::format!(
            "ddtp={ddtp:#x} cqcsr={cq:#x} fqcsr={fq:#x} master={master:#x} status={status} queue_enable={enable} stopped={stopped}"
        )
    }
}

fn iova(base: usize, page: usize) -> usize {
    base + (page - DESC) * PAGE_SIZE
}

fn fail(_: &'static str) -> Failure {
    Failure::Unavailable
}

/// 摆一轮请求：三段描述符（头 → 数据 → 状态）＋可用环那一格。
///
/// `round` 是第几轮：每一轮占可用环的下一格，`avail.idx` 随之推进。只靠一个恒定的
/// `idx` 会在第二轮又把同一条描述符“重新投”一次——那是假的第二轮。
fn arm(pages: &Pages, base: usize, round: u16) {
    descriptor(
        pages,
        0,
        (iova(base, HEADER) as u64, HEADER_BYTES as u32, DESC_NEXT, 1),
    );
    descriptor(
        pages,
        1,
        (
            iova(base, DATA) as u64,
            SECTOR as u32,
            DESC_NEXT | DESC_WRITE,
            2,
        ),
    );
    descriptor(pages, 2, (iova(base, STATUS) as u64, 1, DESC_WRITE, 0));
    pages.put32(HEADER, 0, BLK_IN);
    pages.put32(HEADER, 4, 0);
    pages.put64(HEADER, 8, 0);
    pages.put8(STATUS, 0, 0xFF);
    pages.put16(AVAIL, 4 + round as usize * 2, 0);
    pages.put16(AVAIL, 2, round + 1);
    order::sync();
}

/// 一个描述符：地址 ＋ 长度 ＋ 标志 ＋ 链上的下一项（这不四段就一条链）。
fn descriptor(pages: &Pages, index: usize, link: (u64, u32, u16, u16)) {
    let (address, len, flags, next) = link;
    let at = index * 16;
    pages.put64(DESC, at, address);
    pages.put32(DESC, at + 8, len);
    pages.put16(DESC, at + 12, flags);
    pages.put16(DESC, at + 14, next);
}

/// 一轮里设备写回的可写字节数（正证里要核的那个数）。
pub const fn written_bytes() -> u32 {
    WRITTEN_BYTES
}

/// 块请求成功状态（正证里要核的那个字节）。
pub const fn block_ok() -> u8 {
    BLK_OK
}

/// 那一张盘的花纹：第 `at` 字节 = `at % 256`。
pub fn expected(at: usize) -> u8 {
    (at % 256) as u8
}

/// 把某一页铺成哨兵（负证里核“一字节未动”）。
pub fn sentinel(pages: &Pages, page: usize) {
    pages.fill(page, SENTINEL);
}

/// 读一轮完成：`used.idx`、写回长度、状态字节。
pub fn completion(pages: &Pages) -> (u16, u32, u8) {
    (
        pages.get16(USED, 2),
        pages.get32(USED, 8),
        pages.get8(STATUS, 0),
    )
}

/// 整页是否还是同一个字节（负证里核“一字节未动”）。
pub fn holds_byte(pages: &Pages, page: usize, byte: u8) -> bool {
    (0..PAGE_SIZE).all(|at| pages.get8(page, at) == byte)
}

/// 数据页与盘花纹逐字节相同。
pub fn sector_matches(pages: &Pages) -> bool {
    (0..SECTOR).all(|at| pages.get8(DATA, at) == expected(at))
}

/// 由管理者自己那一枚窗口打响门铃（不经过服务）。
pub fn ring_own(bar: &Dock, caps: &Caps) {
    virtio::ring(bar.view().base(), caps, 0);
}
