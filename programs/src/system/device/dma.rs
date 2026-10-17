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
use crate::system::device::platform::{EndpointDrain, IommuPlatform};
use crate::system::loader::{Image, Loader};
use crate::system::supply::retention::{DeviceRange, Failure, Scope};
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
/// 等服务、等收工的期限（毫秒）。
const WAIT_MS: usize = 2000;
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
    /// 服务已收（走 System 自己的生命周期：`offline` → `isolate` → 确认后才释放）。
    doomed: bool,
    /// 确认之后那一次“自己打响门铃”已经做过。
    proved: bool,
    /// 等设备再推一次的剩几帧。
    prove_left: usize,
    /// 打铃那一时刻的 `used.idx`。
    prove_before: u16,
    reported: bool,
    /// 服务那一台镜像（装配期就从清单里取好，运行期不重查）。
    image: Option<&'static [u8]>,
    kind: Option<env::ProgramKind>,
    /// 开过就失败过：不在旧范围上反复重试。
    failed: bool,
}

/// 一次开始的结果：设备在不在。
pub enum Started {
    /// 本机没有这台设备（普通产品保持行为）。
    Absent,
    /// 已经保留、发布并放开了服务。
    Running,
}

impl Dma {
    /// `image` / `kind` 是那一台端点服务在镜像里的字节与特权级。
    pub const fn new(image: Option<&'static [u8]>, kind: Option<env::ProgramKind>) -> Self {
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
            doomed: false,
            proved: false,
            prove_left: 0,
            prove_before: 0,
            reported: false,
            image,
            kind,
            failed: false,
        }
    }

    pub fn running(&self) -> bool {
        self.service.is_some()
    }

    /// 发现 → 保留 → 发布 → 交付 → 放开服务。
    ///
    /// 不在任何一步阻塞等设备：这是 System 的一帧，测得出来就往下走、测不出来就留着。
    /// 失败一次就**封口**：不在旧范围上反复重试（它已经登记下了，由生命周期收口）。
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
        match self.attempt(loader, supplies) {
            Ok(started) => Ok(started),
            Err(failure) => {
                self.failed = true;
                Err(failure)
            }
        }
    }

    fn attempt(&mut self, loader: &mut Loader, supplies: &mut Supplies) -> Result<Started, Failure> {
        if self.running() {
            return Ok(Started::Running);
        }
        let Some(iommu) = supplies.accounts.token(Name::Page(Page::Region(IOMMU_BASE))) else {
            return Ok(Started::Absent);
        };
        let Some(ecam) = supplies.accounts.token(Name::Page(Page::Region(ECAM_BASE))) else {
            return Ok(Started::Absent);
        };
        let Some(aperture) = supplies.accounts.token(Name::Page(Page::Region(APERTURE_BASE))) else {
            return Ok(Started::Absent);
        };
        self.retention = supplies
            .accounts
            .token(Name::Call(Call::Retain))
            .ok_or(Failure::Denied)?;
        self.aperture = aperture;
        let iommu_dock = Dock::open(iommu).map_err(|_| Failure::Unavailable)?;
        self.base = iommu_dock.view().base();
        let ecam_dock = Dock::open(ecam).map_err(|_| Failure::Unavailable)?;

        // 发现端点：能力表说四个区在哪一根 BAR，端点把那一段落到 aperture 窗口里。
        let requester =
            ecam::find(ecam_dock.view().base(), VENDOR, &BLOCK).ok_or(Failure::Unavailable)?;
        let scan = Ecam::new(ecam_dock.view().base(), requester);
        let (bar_index, caps) = Caps::read(&scan).map_err(|_| Failure::Unavailable)?;
        let endpoint =
            Endpoint::claim(&scan, bar_index, (APERTURE_BASE, APERTURE_SIZE)).map_err(fail)?;
        self.requester = endpoint.requester();
        self.caps = Some(caps);
        let first = ((endpoint.address() - APERTURE_BASE) / PAGE_SIZE as u64) as usize;
        let pages_in_bar = (endpoint.size() as usize).div_ceil(PAGE_SIZE);
        let range = DeviceRange::new(endpoint.address(), endpoint.size()).ok_or(Failure::Denied)?;

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

        // 取这段工作内存：本域一份（建表/读完成），独立保留那一份在 `retain_memory`。
        let pages = Pages::take(self.retention, COUNT).map_err(fail)?;
        let queue = CommandQueue::new(self.base, &pages, COMMANDS, FAULTS, FENCE);
        let platform = IommuPlatform::new(
            range,
            queue,
            Some(EndpointDrain {
                ecam: ecam_dock.view().base(),
                requester,
            }),
        );
        supplies.retention.attach(Scope::new(range, service, Some(Box::new(platform))))?;
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
            table::map_pages(&pages, ROOT, MIDDLE, LEAF, IOVA_BASE, DESC, DEVICE_PAGES)
                .map_err(|_| Failure::Denied)?;
            arm(&pages);
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

        // 两枚窗口：管理者自己那一枚（收口时打响那一次门铃），以及从它收窄出来、
        // 经产品供给窗口那一层交给服务的那一枚（`NativePieEffects`）。
        //
        // **次第一：先把本域那一段图拿下来，再把那一枚交出去**——`ONLY` 的成功
        // 派生是**移交**：交出去之后发送方那枚就不能再 `Open` 了，故图必须在前面。
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
            iova(DESC),
            iova(AVAIL),
            iova(USED),
        ];
        let mut bytes = [0u8; COORD_WORDS * 8];
        for (at, word) in frame.iter().enumerate() {
            bytes[at * 8..at * 8 + 8].copy_from_slice(&word.to_le_bytes());
        }
        Hole::from_raw(writer)
            .push(&bytes, Wait::AtMost(WAIT_MS))
            .map_err(|_| Failure::Pending)?;

        self.iommu = Some(iommu_dock);
        self.ecam = Some(ecam_dock);
        self.bar = Some(bar);
        self.range = Some(range);
        self.pages = Some(pages);
        self.service = Some(service);
        self.team = Some(team);
        env::unit::embark_task(service).map_err(|_| Failure::Pending)?;
        Ok(Started::Running)
    }

    /// 收口：看到设备写回 → 收掉服务（由 System 自己的生命周期隔离并确认）→
    /// 确认之后再用管理者自己那一枚窗口打一次铃，验明设备再也推不动。
    ///
    /// 它**永远返 `Done`**：隔离与释放的节奏归既有生命周期（`retention.sweep`），
    /// 这里只是看结果；返 `Pending` 会把这帧后面的步骤挡住，反而让 `sweep` 跑不到。
    pub fn settle(&mut self, supplies: &mut Supplies) -> ::schedule::Progress {
        use ::schedule::Progress;
        let Some(service) = self.service else {
            return Progress::Done;
        };
        let (used, len, status) = match self.pages.as_ref() {
            Some(pages) => completion(pages),
            None => return Progress::Done,
        };
        if !self.served && used >= 1 {
            self.served = true;
            let sector_ok = self.pages.as_ref().is_some_and(sector_matches);
            self.say(&alloc::format!(
                "system-dma: served used={used} len={len} status={status} sector_ok={sector_ok}"
            ));
        }
        // 服务一收，下面的 `offline(Fault)` → `isolate` → 释放全归既有生命周期。
        if self.served && !self.doomed {
            self.doomed = true;
            let doom = env::room::doom(service).map_err(|error| error.source);
            let slay = self
                .team
                .map(|team| env::unit::slay_team(team).map_err(|error| error.source));
            self.say(&alloc::format!(
                "system-dma: service doomed doom={doom:?} slay={slay:?}"
            ));
        }
        let Some(range) = self.range else {
            return Progress::Done;
        };
        if supplies.retention.occupied().any(|(held, ..)| held == range) {
            return Progress::Done;
        }
        // 确认之后（保留已放下）才做这一手：自己打响那一次门铃。
        if !self.proved {
            self.proved = true;
            if let Some(pages) = self.pages.as_ref() {
                self.prove_before = pages.get16(USED, 2);
                sentinel(pages, DATA);
                pages.put8(STATUS, 0, 0xFF);
                arm(pages);
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
        if !self.reported {
            self.reported = true;
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
                "system-dma: after the death isolation used_moved={moved} sentinel_held={held} status={status} requester={}",
                self.requester,
            ));
        }
        Progress::Done
    }

    fn say(&self, message: &str) {
        crate::debug::put(message);
    }
}

fn iova(page: usize) -> usize {
    IOVA_BASE + (page - DESC) * PAGE_SIZE
}

fn fail(_: &'static str) -> Failure {
    Failure::Unavailable
}

/// 摆一轮请求：三段描述符（头 → 数据 → 状态）＋可用环那一格。
fn arm(pages: &Pages) {
    descriptor(pages, 0, (iova(HEADER) as u64, HEADER_BYTES as u32, DESC_NEXT, 1));
    descriptor(
        pages,
        1,
        (iova(DATA) as u64, SECTOR as u32, DESC_NEXT | DESC_WRITE, 2),
    );
    descriptor(pages, 2, (iova(STATUS) as u64, 1, DESC_WRITE, 0));
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
