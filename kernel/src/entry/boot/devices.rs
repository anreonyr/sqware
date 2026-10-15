use crate::core::memory::nest::Access;
use crate::core::work::mail::{self, Mail};
use crate::core::work::pend::{self, Permission, SourceAuthorization};
use crate::entry::boot::resources::Registry;
use crate::machine::board;
use env::{Mark, Name, Page, PieFail, TaskId};

pub(crate) fn register(registry: &mut Registry) -> Result<(), PieFail> {
    let dtb = board::info().dtb();
    let fdt = board::device_tree().ok_or(PieFail::Denied)?;
    for node in fdt.all_nodes() {
        if exempt(node.name) {
            continue;
        }
        if let Some(reg) = node.reg() {
            for r in reg {
                grant_region(registry, r.starting_address.addr(), r.size)?;
            }
        }
        // PCI 宿主的 MMIO 开口不在 `reg` 里：它在 `ranges`（子空间→父地址）那一条
        // 属性上。BAR 就落在那几段窗口里，故引导期按 `ranges` 逐个登记，
        // 让窗口里的页与 FDT 声明的区一样有自己的门闩。
        grant_pci_windows(registry, &fdt, node)?;
    }
    let meta = mail::pole::region(dtb.base, dtb.size, TaskId::new(0))?;
    let root = pend::Pie::try_new(
        Mail::Pole(meta),
        Mark::NONE,
        Permission::FETCH | Permission::VEST,
        None,
    )?;
    registry
        .register(Name::Page(Page::Dtb), root)
        .map_err(|e| e.into_parts().0)?;
    if let Some(initrd) = board::info().initrd() {
        let meta = mail::pole::region(initrd.base, initrd.size, TaskId::new(0))?;
        let root = pend::Pie::try_new(
            Mail::Pole(meta),
            Mark::NONE,
            Permission::FETCH | Permission::VEST,
            None,
        )?;
        registry
            .register(Name::Page(Page::Initrd), root)
            .map_err(|e| e.into_parts().0)?;
    }
    Ok(())
}

fn grant_region(registry: &mut Registry, base: usize, size: Option<usize>) -> Result<(), PieFail> {
    let Some(size) = size else {
        return Ok(());
    };
    if base == 0 || size == 0 {
        return Ok(());
    }
    let meta = mail::pole::region(base, size, TaskId::new(0))?;
    let source = meta.source();
    let root = pend::Pie::try_new(
        Mail::Pole(meta.clone()),
        Mark::NONE,
        Permission::FETCH | Permission::STORE | Permission::VEST | Permission::ONLY,
        None,
    )?;
    // 借来的区也算一份来源：把授权记录接上，持有者就能取得**自己那一小段**子窗口
    // （而不是只能攞整段）。与 `Unseal::Pole` 接手私有根同一套，不另造旁路。
    // 上限就是 MMIO 的读写两位：设备区不开执行映射。
    let authorization =
        SourceAuthorization::new(source, Access::READ | Access::WRITE, TaskId::new(0))?;
    meta.attach_source(&authorization);
    authorization.adopt(&meta, root.token(), root.grant().clone())?;
    registry
        .register(Name::Page(Page::Region(base as u64)), root)
        .map_err(|e| e.into_parts().0)
}

/// 是不是 PCI 宿主：`device_type` 必须**正好**是 `pci`。
///
/// 名字叫什么都可以，不按名字认；`pcie`、`pci-something` 这些前缀相同的取值也不收——
/// 它们的 `ranges` 未必是 PCI 宿主那一套语意。
pub(crate) fn is_pci_host(node: fdt::node::FdtNode) -> bool {
    node.property("device_type")
        .and_then(|property| property.as_str())
        == Some("pci")
}

/// PCI 宿主的 MMIO 窗口：可支持的形状就登记，不支持的当场拒。
fn grant_pci_windows(
    registry: &mut Registry,
    fdt: &fdt::Fdt,
    node: fdt::node::FdtNode,
) -> Result<(), PieFail> {
    if !is_pci_host(node) {
        return Ok(());
    }
    let dram = board::info().dram;
    let dram_edge = dram.base.checked_add(dram.size).ok_or(PieFail::Denied)?;
    for (base, size) in memory_windows(fdt, node)? {
        // MMIO 窗口不能压在 DRAM 上：这比“先登记再看”更早地把错形状拦住。
        let edge = base.checked_add(size).ok_or(PieFail::Denied)?;
        if base < dram_edge && dram.base < edge {
            return Err(PieFail::Denied);
        }
        grant_region(registry, base, Some(size))?;
    }
    Ok(())
}

fn cells(node: fdt::node::FdtNode, name: &str) -> Option<usize> {
    node.property(name).and_then(|property| property.as_usize())
}

/// 这一节点的**父节点**（靠“谁的 children 里有它”找）。
///
/// fdt 那一层不公开父链接，故父的单元数不能猜：查不到就当场拒，
/// 宁可不开这条窗口，也不能按根的数目先登记一段错的物理区。
///
/// 认节点用**借用切片的身份**（同一条属性值在 DTB 里的地址），不用名字或内容：
/// 两个不同父节点下可以有同名、且 `ranges` 字节完全一样的子节点，
/// 按内容匹配就会把后来的那个认到先出现的父上去，于是用错父的 `#address-cells`。
fn parent_of<'b, 'a>(
    fdt: &'b fdt::Fdt<'a>,
    node: fdt::node::FdtNode<'b, 'a>,
) -> Option<fdt::node::FdtNode<'b, 'a>> {
    let ranges = node.property("ranges")?.value;
    fdt.all_nodes().find(|candidate| {
        candidate.children().any(|child| {
            child
                .property("ranges")
                .is_some_and(|property| core::ptr::eq(property.value, ranges))
        })
    })
}

/// PCI 宿主的 `ranges` 里那几段**内存空间**窗口的父地址与长度。
///
/// 一条占 `child + parent + size` 格（都是**四字节格**）：
/// 子地址那几格（`child` = 宿主自己的 `#address-cells`，PCI 通常 3——
/// 首格是空间码）→ 父地址那几格（`parent` = 父节点的 `#address-cells`）
/// → 长度那几格（`size` = 宿主自己的 `#size-cells`）。
/// 只取内存空间（空间码位 25:24 为 10 / 11）；IO 空间不是可映射的 MMIO 窗口。
/// 形状不是这一套（格数不合、字节长不成整条、坐标或跨度溢出）就报错，
/// 不静默退回空表。
pub(crate) fn ranges_windows(
    value: &[u8],
    child: usize,
    parent: usize,
    size: usize,
) -> Result<alloc::vec::Vec<(usize, usize)>, PieFail> {
    if child != 3 || !(1..=2).contains(&parent) || !(1..=2).contains(&size) {
        return Err(PieFail::Denied);
    }
    let width = child + parent + size;
    if !value.len().is_multiple_of(width * 4) {
        return Err(PieFail::Denied);
    }
    let count = value.len() / (width * 4);
    let mut windows = alloc::vec::Vec::new();
    windows.try_reserve(count).map_err(|_| PieFail::OoM)?;
    for entry in value.chunks_exact(width * 4) {
        let cell = |at: usize| -> u64 {
            let bytes: [u8; 4] = entry[at * 4..at * 4 + 4].try_into().expect("whole cell");
            u32::from_be_bytes(bytes) as u64
        };
        // 空间码在子地址的首格；不是内存空间就不是可映射窗口。
        if (cell(0) >> 24) & 0x3 < 2 {
            continue;
        }
        let mut address = 0u64;
        for at in child..child + parent {
            address = address.checked_shl(32).ok_or(PieFail::Denied)? | cell(at);
        }
        let mut length = 0u64;
        for at in child + parent..width {
            length = length.checked_shl(32).ok_or(PieFail::Denied)? | cell(at);
        }
        let base = usize::try_from(address).map_err(|_| PieFail::Denied)?;
        let span = usize::try_from(length).map_err(|_| PieFail::Denied)?;
        if base == 0 || span == 0 {
            continue;
        }
        base.checked_add(span).ok_or(PieFail::Denied)?;
        windows.push((base, span));
    }
    Ok(windows)
}

pub(crate) fn memory_windows(
    fdt: &fdt::Fdt,
    node: fdt::node::FdtNode,
) -> Result<alloc::vec::Vec<(usize, usize)>, PieFail> {
    let Some(ranges) = node.property("ranges") else {
        return Ok(alloc::vec::Vec::new());
    };
    let child = cells(node, "#address-cells").ok_or(PieFail::Denied)?;
    let size = cells(node, "#size-cells").ok_or(PieFail::Denied)?;
    let parent = parent_of(fdt, node).ok_or(PieFail::Denied)?;
    let parent = cells(parent, "#address-cells").ok_or(PieFail::Denied)?;
    ranges_windows(ranges.value, child, parent, size)
}

fn exempt(name: &str) -> bool {
    let stem = name.split('@').next().unwrap_or(name);
    matches!(stem, "memory" | "clint")
}
