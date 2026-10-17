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

use alloc::boxed::Box;

use env::{AcquireSource, Call, Mark, Name, PAGE_SIZE, Page, Permission, PieToken, TaskId, Wait};
use system_api::supply::LeaseId;

use resource::dock::Dock;
use resource::raw::Hole;

use crate::driver::iommu::{
    context::DeviceContext,
    order,
    pages::Pages,
    queue::CommandQueue,
    regs, table,
};
use crate::driver::pci::ecam::{self, Ecam};
use crate::driver::pci::endpoint::Endpoint;
use crate::driver::pci::virtio::{self, Caps};
use crate::system::app::supplies::Supplies;
use crate::system::device::fault::FaultOnce;
use crate::system::device::platform::{EndpointDrain, IommuPlatform};
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
/// 交给服务的那一帧坐标的字数。
const COORD_WORDS: usize = 9;
/// 确认之后等设备再推一次的帧数（System 的一帧一帧，不阻塞）。
const PROVE_FRAMES: usize = 8;

/// 产品 DMA 管理者：一次只跑一台端点。
pub struct Dma {
    iommu: Option<Dock>,
    ecam: Option<Dock>,
    /// System 自己那一枚 BAR 窗口：收口时由它打响那一次门铃（不经过任何服务）。
    bar: Option<Dock>,
    retention: PieToken,
    aperture: PieToken,
    base: usize,
    range: Option<DeviceRange>,
    pages: Option<Pages>,
    caps: Option<Caps>,
    requester: u16,
    service: Option<TaskId>,
    /// 服务那一 Team（收工时按 Team 收，与 `ruin` 同一条路）。
    team: Option<env::TeamId>,
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
    /// 这一景走哪一条收口：`false` = 显式 `Close::Normal`，`true` = 收服务走
    /// `offline(Fault)`。两者停链相同，差别只在“谁先关闸”。
    fault: bool,
    /// 这一景是否装一次“第一次停住答 Pending”的注入（只故障景为真）。
    inject: bool,
    /// 服务那一台镜像（装配期就从清单里取好，运行期不重查）。
    image: Option<&'static [u8]>,
    kind: Option<env::ProgramKind>,
    /// 开过就失败过：不在旧范围上反复重试。
    failed: bool,
    /// 确认后验也报过了。
    done: bool,
}

/// 一次开始的结果：设备在不在。
pub enum Started {
    /// 本机没有这台设备（普通产品保持行为）。
    Absent,
    /// 已经保留、发布并放开了服务。
    Running,
}

impl Dma {
    /// `image` / `kind` 是那一台端点服务在镜像里的字节与特权级；`fault` / `inject`
    /// 由这一景的装配给出（普通产品为假）。
    pub const fn new(
        image: Option<&'static [u8]>,
        kind: Option<env::ProgramKind>,
        fault: bool,
    ) -> Self {
        Self {
            iommu: None,
            ecam: None,
            bar: None,
            retention: PieToken::NONE,
            aperture: PieToken::NONE,
            base: 0,
            range: None,
            pages: None,
            caps: None,
            requester: 0,
            service: None,
            team: None,
            served: false,
            closing: false,
            pending_seen: false,
            proved: false,
            prove_left: 0,
            prove_before: 0,
            reported: false,
            fault,
            inject: fault,
            image,
            kind,
            failed: false,
            done: false,
        }
    }

    pub fn running(&self) -> bool {
        self.service.is_some()
    }

    /// 发现 → 保留 → 发布 → 交付 → 放开服务。
    ///
    /// 不在任何一步阻塞等设备：这是 System 的一帧，测得出来就往下走、测不出来就留着。
    /// 起手失败一次就**封口不再重试**，但把已经拿到的东西**主动**交回去
    /// （见 [`Dma::rollback`]）——不指望一个还没放开的任务自己死。
    pub fn begin(
        &mut self,
        loader: &mut Loader,
        supplies: &mut Supplies,
    ) -> Result<Started, Failure> {
        if self.failed {
            return Ok(Started::Absent);
        }
        if self.running() {
            return Ok(Started::Running);
        }
        match self.stage(loader, supplies) {
            Ok(started) => Ok(started),
            Err(failure) => {
                self.failed = true;
                self.rollback(supplies);
                Err(failure)
            }
        }
    }

    /// 起手失败的回退：**主动**把子任务收进既有死亡路径、把范围关成 `Fault`
    /// （没发布过的登记直接结清，发布过的那些由平台确认后释放），本地所有者
    /// （`pages` / 映射 / 目录表）留在 `self` 上让 `settle` 接着报、接着收。
    fn rollback(&mut self, supplies: &mut Supplies) {
        if let Some(service) = self.service.take() {
            let _ = env::room::doom(service);
        }
        if let Some(team) = self.team.take() {
            let _ = env::unit::slay_team(team);
        }
        if let Some(range) = self.range {
            if !self.closing {
                self.closing = true;
                let _ = supplies.retention.close(range, Close::Fault);
                self.say("system-dma: startup failed, range closed as fault");
            }
        }
    }

    fn stage(&mut self, loader: &mut Loader, supplies: &mut Supplies) -> Result<Started, Failure> {
        if self.running() {
            return Ok(Started::Running);
        }
        let Some(iommu) = supplies.accounts.token(Name::Page(Page::Region(IOMMU_BASE))) else {
            return Ok(Started::Absent);
        };
        // 两轮共用同一片寄存器窗与 ECAM：只开一次图（第一轮那一开）。
        if self.iommu.is_none() {
            let Some(ecam) = supplies.accounts.token(Name::Page(Page::Region(ECAM_BASE))) else {
                return Ok(Started::Absent);
            };
            let Some(aperture) =
                supplies.accounts.token(Name::Page(Page::Region(APERTURE_BASE)))
            else {
                return Ok(Started::Absent);
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
            Endpoint::claim(&scan, bar_index, (APERTURE_BASE, APERTURE_SIZE)).map_err(fail)?;
        self.requester = endpoint.requester();
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
            .build(Image {
                bytes: image,
                kind,
            })
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
        let queue = CommandQueue::new(self.base, &pages, COMMANDS, FAULTS, FENCE);
        let inner = IommuPlatform::new(
            range,
            queue,
            Some(EndpointDrain {
                ecam: ecam_base,
                requester,
                bar: bar_base,
                caps,
            }),
        );
        // 只故障景：装一层“第一次停住答 `Pending`”，验“确认失败仍保留”。
        let platform: Box<dyn Platform> = if self.inject {
            Box::new(FaultOnce::new(inner))
        } else {
            Box::new(inner)
        };
        supplies.retention.attach(Scope::new(range, service, Some(platform)))?;
        let id = supplies.retain_memory(
            range,
            AcquireSource::Source {
                token: pages.source(),
                offset: 0,
                count: COUNT,
            },
        )?;
        // 发布：表与队列都布好、`IOFENCE` 拿到完成记录之后才接上设备目录表。
        let ddt = pages.physical(DDT);
        let root = pages.physical(ROOT);
        let context = DeviceContext::first_stage(root);
        let requester = self.requester as usize;
        supplies.retention.publish(id, |_| {
            for (index, word) in context.words().iter().enumerate() {
                pages.put64(DDT, requester * 64 + index * 8, *word);
            }
            table::map_pages(&pages, ROOT, MIDDLE, LEAF, base, DESC, DEVICE_PAGES)
                .map_err(|_| Failure::Denied)?;
            arm(&pages, base);
            let mut queues = CommandQueue::new(self.base, &pages, COMMANDS, FAULTS, FENCE);
            queues.enable().map_err(|_| Failure::Unavailable)?;
            queues.invalidate_all();
            if !queues.fence() {
                return Err(Failure::Pending);
            }
            order::sync();
            regs::ddtp_set(self.base, regs::DDTP_MODE_1LVL, Some(ddt))
                .map_err(|_| Failure::Unavailable)?;
            Ok(())
        })?;

        // 把那一枚交出去（`ONLY`：发送方那一枚就此不可再 `Open`，图已在上面拿下）。
        let root_window = PrivateRoot::derive(
            RootAssignment {
                shared: mine,
                index: 0,
                window: Some((first, pages_in_bar)),
            },
            RootRequest {
                id: LeaseId(1),
                origin: Origin {
                    requester: env::unit::self_id(),
                    authority: mine.get(),
                },
                owner: service,
                permission: DRIVER_SUBSET,
            },
        )
        .map_err(|_| Failure::Denied)?;
        let seed = root_window.token();
        crate::debug::put(&alloc::format!(
            "system-dma: endpoint requester={} bar={:#x} size={:#x} window={first}+{pages_in_bar} seed={}",
            self.requester,
            endpoint.address(),
            endpoint.size(),
            seed.get(),
        ));

        // 坐标只经那一枚孔过去：种子、页数、能力表四格、三段 IOVA。
        // 服务还按着（没 embark），故是一个**非阻塞的一次尝试**：装不下就当场失败
        // 回退，不在 System 的一帧里 park。
        let (common, notify, multiplier, device) = caps.parts();
        let writer = ipc::session::establish::give(service, COORD_MARK)
            .map_err(|_| Failure::Conflict)?;
        let frame = [
            seed.get(),
            pages_in_bar,
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
        Ok(Started::Running)
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
            if !self.fault {
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
                arm(pages, IOVA_BASE);
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
                self.requester,
            ));
            // 到这里本域对那一段工作内存的**最后一次访问**已经过完：图与匿名来源
            // 可以收 — 归属侧是 System 自己，没有别人的窗口从这里派生过。
            if let Some(pages) = self.pages.take() {
                let _ = pages.close();
            }
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
        let master = self
            .ecam
            .as_ref()
            .map(|dock| {
                Ecam::new(dock.view().base(), self.requester).read16(ecam::COMMAND)
                    & ecam::COMMAND_MASTER
            })
            .unwrap_or(ecam::COMMAND_MASTER);
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
fn arm(pages: &Pages, base: usize) {
    descriptor(pages, 0, (iova(base, HEADER) as u64, HEADER_BYTES as u32, DESC_NEXT, 1));
    descriptor(
        pages,
        1,
        (iova(base, DATA) as u64, SECTOR as u32, DESC_NEXT | DESC_WRITE, 2),
    );
    descriptor(pages, 2, (iova(base, STATUS) as u64, 1, DESC_WRITE, 0));
    pages.put32(HEADER, 0, BLK_IN);
    pages.put32(HEADER, 4, 0);
    pages.put64(HEADER, 8, 0);
    pages.put8(STATUS, 0, 0xFF);
    pages.put16(AVAIL, 4, 0);
    pages.put16(AVAIL, 2, 1);
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
