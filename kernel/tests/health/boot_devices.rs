//! PCI 宿主 `ranges` 的解析：**格**怎么分、坏形状怎么拒，以及选父与 `device_type` 的判据。
//!
//! 这一段不能靠 QEMU virt 那种“父址恰好等于长度”的 DTB 证明：那里分错字段也看不出。
//! 故这里用非 identity、且长度与父址不同的合成表逐项对，并覆盖 32/64 位、
//! 截断、溢出与不支持的格数；选父与 `device_type` 各用一份真 DTB 夹具（见
//! 同目录 `.dts`，`dtc` 可重建，`.dtb` 与用例同级）。

use crate::entry::boot::devices::{is_pci_host, memory_windows, ranges_windows};
use alloc::vec::Vec;
use env::PieFail;

/// 两个不同父节点下各有一个同名 `pci@30000000`，`ranges` 字节完全相同，
/// 只有父的 `#address-cells` 不同（1 与 2）。
///
/// 168 B = 42 格**同时**被 6 格/条与 7 格/条整除：认错父不会报错，只会静默解析成错窗口。
/// `dtc -I dts -O dtb -o parent_of.dtb parent_of.dts`
/// sha256 = 14c8e1783bb608e174b73073ca14d884e4f0d4ad09b493ea9c42a1cae81ec592
const PARENT_OF: &[u8] = include_bytes!("boot_devices/parent_of.dtb");

/// 三个前缀都是 `pci` 的 `device_type`：`pci` / `pcie` / `pci-something`。
/// sha256 = 63f2487d43ecc194c677e5e1dddcaac25047b3cd0895e95e8fcde300fd60774e
const DEVICE_TYPE: &[u8] = include_bytes!("boot_devices/device_type.dtb");

/// 把若干条 `ranges` 写成大端四字节格。
fn words(cells: &[u32]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for cell in cells {
        bytes.extend_from_slice(&cell.to_be_bytes());
    }
    bytes
}

/// 32 位内存 + 64 位内存 + 一段 IO：只有前两条是窗口，地址与长度都不是同一个数。
pub fn accept() {
    ranges_shape();
    parent_identity();
    device_type_exactness();
}

/// `ranges` 的格与形状：分字段、拒坏形状。
fn ranges_shape() {
    let table = words(&[
        // 32-bit memory：子址 (space, 0, 0x8000_0000)，父址 0x6000_0000，长 0x10_0000
        0x0200_0000,
        0x0000_0000,
        0x8000_0000,
        0x0000_0000,
        0x6000_0000,
        0x0000_0000,
        0x0010_0000,
        // 64-bit memory：子址 (space, 4, 0) = PCI 0x4_0000_0000，父址 0x7000_0000，长 0x20_0000
        0x0300_0000,
        0x0000_0004,
        0x0000_0000,
        0x0000_0000,
        0x7000_0000,
        0x0000_0000,
        0x0020_0000,
        // IO space：不是可映射窗口，既不进表也不报错
        0x0100_0000,
        0x0000_0000,
        0x0000_0000,
        0x0000_0000,
        0x0300_0000,
        0x0000_0000,
        0x0001_0000,
    ]);
    assert_eq!(
        ranges_windows(&table, 3, 2, 2).unwrap(),
        alloc::vec![(0x6000_0000, 0x10_0000), (0x7000_0000, 0x20_0000)],
        "父址与尾部长度的字段不能错位"
    );

    // 只剩 IO：形状认得，只是没有可映射窗口。
    let io = words(&[0x0100_0000, 0, 0, 0, 0x0300_0000, 0, 0x10000]);
    assert!(ranges_windows(&io, 3, 2, 2).unwrap().is_empty());

    // 截断：字节长不成整条 ⇒ 拒。
    let mut cut = table.clone();
    cut.truncate(table.len() - 4);
    assert_eq!(ranges_windows(&cut, 3, 2, 2), Err(PieFail::Denied));

    // 空表：既不是合法的一条“零段表”，也不当“解析不出来”报错——没有窗口。
    assert!(ranges_windows(&[], 3, 2, 2).unwrap().is_empty());

    // 父址 + 长度溢出 `usize`：拒，不绕回。
    let overflow = words(&[
        0x0200_0000,
        0,
        0x8000_0000,
        0xFFFF_FFFF,
        0xFFFF_FFFF,
        0,
        0x2,
    ]);
    assert_eq!(ranges_windows(&overflow, 3, 2, 2), Err(PieFail::Denied));

    // 不支持的格数：子址不是 3 格（PCI 子址首格是空间码），或父/长度格越界。
    assert_eq!(ranges_windows(&table, 2, 2, 2), Err(PieFail::Denied));
    assert_eq!(ranges_windows(&table, 3, 3, 2), Err(PieFail::Denied));
    assert_eq!(ranges_windows(&table, 3, 2, 3), Err(PieFail::Denied));
    assert_eq!(ranges_windows(&table, 3, 0, 2), Err(PieFail::Denied));
}

/// 同名、`ranges` 字节相同的两个子节点，必须各自认到**自己那个**父。
///
/// 认错父就会拿错 `#address-cells`，把同一条 `ranges` 切成别的窗口：
/// `/real` 下那个是 7 格/条（父 2 格），`/fake` 下那个是 6 格/条（父 1 格）。
fn parent_identity() {
    let fdt = fdt::Fdt::new(PARENT_OF).expect("boot_devices: parse the parent fixture");
    let real = fdt
        .find_node("/real/pci@30000000")
        .expect("boot_devices: /real child");
    let fake = fdt
        .find_node("/fake/pci@30000000")
        .expect("boot_devices: /fake child");
    assert_eq!(
        memory_windows(&fdt, real).unwrap(),
        alloc::vec![
            (0x4000_0000, 0x4000_0000),
            (0x4_0000_0000, 0x4_0000_0000),
            (0x5000_0000, 0x2000_0000),
            (0x7000_0000, 0x1000_0000),
        ],
        "boot_devices: /real must read its own parent's 7-cell split"
    );
    assert_eq!(
        memory_windows(&fdt, fake).unwrap(),
        alloc::vec![(0x0200_0000, 0x5000_0000)],
        "boot_devices: /fake must read its own parent's 6-cell split"
    );
    // 两个子节点同名、`ranges` 字节完全相同，只有切片身份不同——这正是旧规则认错的入口。
    let real_ranges = real.property("ranges").unwrap().value;
    let fake_ranges = fake.property("ranges").unwrap().value;
    assert_eq!(
        real_ranges, fake_ranges,
        "boot_devices: the fixture's two ranges really are byte-identical"
    );
    assert!(
        !core::ptr::eq(real_ranges, fake_ranges),
        "boot_devices: identical bytes must still be distinct slices"
    );
}

/// `device_type` 只认正好是 `pci` 的；前缀相同的 `pcie` / `pci-something` 不收。
fn device_type_exactness() {
    let fdt = fdt::Fdt::new(DEVICE_TYPE).expect("boot_devices: parse the device_type fixture");
    let value = |path: &str| -> Vec<u8> {
        fdt.find_node(path)
            .unwrap_or_else(|| panic!("boot_devices: {path}"))
            .property("device_type")
            .expect("boot_devices: device_type")
            .value
            .to_vec()
    };
    // 三个都以前缀 `pci` 开头：旧规则会把三个都当成宿主，故这条用例能区分。
    assert_eq!(value("/pci@40000000"), b"pci\0");
    assert_eq!(value("/pcie@41000000"), b"pcie\0");
    assert_eq!(value("/bridge@42000000"), b"pci-something\0");
    assert!(
        is_pci_host(fdt.find_node("/pci@40000000").unwrap()),
        "boot_devices: device_type = \"pci\" is a host"
    );
    assert!(
        !is_pci_host(fdt.find_node("/pcie@41000000").unwrap()),
        "boot_devices: device_type = \"pcie\" must not be a host"
    );
    assert!(
        !is_pci_host(fdt.find_node("/bridge@42000000").unwrap()),
        "boot_devices: device_type = \"pci-something\" must not be a host"
    );
}
