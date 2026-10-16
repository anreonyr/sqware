//! User memory: source holds, mappings and the program heap.

pub use env::PAGE_SIZE;
#[path = "memory/reference.rs"]
mod reference;
#[path = "memory/view.rs"]
mod view;
use env::wire::Field;
use env::{
    AcquireSource, Allocation, FlatReference, MemoryFail, MemoryResult, PieToken, Placement,
    ReferenceRange, TeamId, ViewControl, ViewId, ViewSelection, VirtAddr,
};
pub use reference::Reference;
pub use view::View;

/// 当前任务的身份（引用表的作用域就是它）。
///
/// 单列成一扇门，是为了让身体（全应用层）只认这一处；宿主机夹具可自己给出身份。
pub fn current_task() -> env::TaskId {
    crate::unit::task::self_id()
}

#[path = "memory/allocator.rs"]
mod allocator;

/// 取一份来源持有（登记在**调用任务**的引用表里）。
pub fn acquire(source: AcquireSource, retention: PieToken) -> MemoryResult<FlatReference> {
    env::memory::acquire(source, retention)
}

/// 解除这份持有：不隐式 Detach，不等待其他引用，不分配。
pub fn release(reference: FlatReference) -> MemoryResult<()> {
    env::memory::release(reference)
}

/// 在目标域里预留地址并建立翻译关系，返回 `(关系句柄, 视图起点)`。
///
/// 选址与范围都在**栈上**编码成定长描述，内核一次快照读走，不经任何共享缓冲。
pub fn attach(
    range: ReferenceRange,
    team: TeamId,
    placement: Placement,
    access: u64,
    control: ViewControl,
) -> MemoryResult<(ViewId, usize)> {
    let mut range_bytes = [0; ReferenceRange::WIDTH];
    let mut placement_bytes = [0; Placement::WIDTH];
    range.store(&mut range_bytes);
    placement.store(&mut placement_bytes);
    env::memory::attach(
        VirtAddr::new(range_bytes.as_ptr() as usize),
        team,
        VirtAddr::new(placement_bytes.as_ptr() as usize),
        access,
        control,
    )
    .map(|(view, address)| (view, address.get()))
}

/// 按身份安全撤销选定的映射区间。
pub fn detach(team: TeamId, view: ViewId, selection: ViewSelection) -> MemoryResult<()> {
    env::memory::detach(team, view, selection)
}

/// 修改选定区间的逻辑权限。
pub fn set_access(
    team: TeamId,
    view: ViewId,
    selection: ViewSelection,
    access: u64,
) -> MemoryResult<()> {
    env::memory::set_access(team, view, selection, access)
}

/// 把一份**独立保留**持有那一页窗口的稳定物理页起址读出来（每页一个 8 字节小端字）。
///
/// `retention` 是调用者当前活着的真保留授权，内核当场核过；
/// `range` 是调用者自己的那枚 retained 引用窗口。`out` 必须正好装下描述里的页数，
/// 不够或多了都当场拒，不猜一个截断的长度。
pub fn traverse(retention: PieToken, range: ReferenceRange, out: &mut [u64]) -> MemoryResult<()> {
    if out.len() != range.count {
        return Err(env::make_fail(MemoryFail::NotAligned));
    }
    let mut range_bytes = [0; ReferenceRange::WIDTH];
    range.store(&mut range_bytes);
    env::memory::traverse(
        retention,
        VirtAddr::new(range_bytes.as_ptr() as usize),
        VirtAddr::new(out.as_mut_ptr() as usize),
    )
}

/// 把一份来源窗口里的一页承载交进另一份窗口的空槽。
///
/// 两个页号各相对**自己那份引用**的窗口；内核当场按窗口解析成来源坐标，
/// 不在这里预加任何偏移。两边都要求当前任务此刻是那份来源的管理者。
pub fn transfer(
    from: FlatReference,
    from_page: usize,
    to: FlatReference,
    to_page: usize,
) -> MemoryResult<()> {
    env::memory::transfer(from, from_page, to, to_page)
}

/// 便利分配的前半：Acquire → Attach。返回 `(地址, 关系句柄, 仍持有的那份引用)`。
///
/// 拿到关系之后**必须**把那份引用解掉；解失败就把它记在调用方自己的元数据里
/// 重试（句柄是表作用域的，只有调用任务能解）。
pub fn attach_alloc(
    size: usize,
    control: ViewControl,
) -> MemoryResult<(usize, ViewId, FlatReference)> {
    let bytes = size
        .max(1)
        .checked_next_multiple_of(PAGE_SIZE)
        .ok_or_else(|| env::make_fail(MemoryFail::NotAligned))?;
    let allocation =
        Allocation::new(bytes, PAGE_SIZE).ok_or_else(|| env::make_fail(MemoryFail::NotAligned))?;
    let count = bytes / PAGE_SIZE;
    // 选址先在 Acquire 之前备好：它不依赖那份引用，失败了就不能已经取了引用。
    let placement =
        Placement::lowest(PAGE_SIZE).ok_or_else(|| env::make_fail(MemoryFail::NotAligned))?;
    let reference = acquire(AcquireSource::Allocation(allocation), PieToken::NONE)?;
    let Some(range) = ReferenceRange::new(reference, 0, count) else {
        discard(reference);
        return Err(env::make_fail(MemoryFail::NotAligned));
    };
    match attach(range, TeamId::new(0), placement, 6, control) {
        Ok((view, address)) => Ok((address, view, reference)),
        Err(error) => {
            // 没落地：这一份引用只剩清理责任，交回不成功就随任务尽头结清。
            discard(reference);
            Err(error)
        }
    }
}

/// 没落地时的清理：这份引用只剩“从自己表里摘掉”一件事。
/// 解不掉也只是留着，由 Task 尽头统一结清（引用表是调用任务的作用域）。
fn discard(reference: FlatReference) {
    release(reference).expect("fresh caller reference cleanup");
}

/// 私有便利分配（`Caller` 控制；TLS 这类一段关系只归发起线程的用法）。
pub(crate) fn allocate_raw(size: usize) -> MemoryResult<(usize, ViewId, FlatReference)> {
    attach_alloc(size, ViewControl::Caller)
}

/// 跨 Task 归还的便利分配（`SpaceMembers` 控制；用户堆整区）。
pub(crate) fn allocate_shared_raw(size: usize) -> MemoryResult<(usize, ViewId, FlatReference)> {
    attach_alloc(size, ViewControl::SpaceMembers)
}

/// 在目标域里装一段映射：`pole=NONE` 是惰性匿名来源，否则是 Pole 授权范围内的窗口。
///
/// 返回 `(关系句柄, 视图起点)`；归还走 [`detach`] 按准确身份执行。
pub fn map(
    team: TeamId,
    at: usize,
    size: usize,
    pole: PieToken,
    offset: usize,
    flags: u64,
    control: ViewControl,
) -> MemoryResult<View> {
    if size == 0 || !size.is_multiple_of(PAGE_SIZE) || !offset.is_multiple_of(PAGE_SIZE) {
        return Err(env::make_fail(MemoryFail::NotAligned));
    }
    let count = size / PAGE_SIZE;
    // 选址与范围描述都不依赖引用：先在 Acquire 之前备好，
    // 失败时还什么都没取，就不会漏一份引用。
    let placement = if at == 0 {
        Placement::lowest(PAGE_SIZE)
    } else {
        Placement::exact(VirtAddr::new(at), PAGE_SIZE)
    }
    .ok_or_else(|| env::make_fail(MemoryFail::NotAligned))?;
    let source = match pole {
        PieToken::NONE => {
            if offset != 0 {
                return Err(env::make_fail(MemoryFail::Denied));
            }
            AcquireSource::Allocation(
                Allocation::new(size, PAGE_SIZE)
                    .ok_or_else(|| env::make_fail(MemoryFail::NotAligned))?,
            )
        }
        token => AcquireSource::Source {
            token,
            offset: offset / PAGE_SIZE,
            count,
        },
    };
    let mut reference = Reference::acquire(source, PieToken::NONE)?;
    let mut view = match reference.attach(team, 0, count, placement, flags, control) {
        Ok(view) => view,
        Err(error) => {
            reference.release().expect("fresh caller reference cleanup");
            return Err(error);
        }
    };
    if reference.release().is_err() {
        view.pending = Some(reference);
    }
    Ok(view)
}

pub fn allocate(size: usize) -> MemoryResult<View> {
    let bytes = size
        .max(1)
        .checked_next_multiple_of(PAGE_SIZE)
        .ok_or_else(|| env::make_fail(MemoryFail::NotAligned))?;
    map(
        TeamId::new(0),
        0,
        bytes,
        PieToken::NONE,
        0,
        6,
        ViewControl::Caller,
    )
}
