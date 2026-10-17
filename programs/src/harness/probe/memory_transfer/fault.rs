//! 故障任务那一边：花纹与整页校验。
//!
//! 花纹按页分：同一页搬走再搬回来，内容逐字节不变；不同页的花纹不同，故「落错了页」
//! 当场失败，而不是「都能读出一个数」。内容判据是任务内的逐字节断言；
//! 返回的和只是正常完成载荷，不是内容唯一性的证明。

use env::PAGE_SIZE;

/// 第 `page` 页第 `offset` 字节该是什么。
pub const fn at(page: usize, offset: usize) -> u8 {
    (offset as u8)
        .wrapping_mul(31)
        .wrapping_add(page as u8)
        .wrapping_add(7)
}

/// 把整页写成第 `page` 个花纹。
pub fn fill(page: usize, base: usize) {
    for offset in 0..PAGE_SIZE {
        // SAFETY: 调用方保证这一页在本任务的映射里（首字节会走一次真实缺页）。
        unsafe { core::ptr::write_volatile((base + offset) as *mut u8, at(page, offset)) };
    }
}

/// 第 `page` 页整页花纹的和（控制者按它核对故障任务交回的读数）。
pub fn sum(page: usize) -> u64 {
    let mut total = 0u64;
    for offset in 0..PAGE_SIZE {
        total += at(page, offset) as u64;
    }
    total
}

/// 故障任务的身子：**头一个字节就是那一次真正的访存**（承载不在就等在这里），
/// 醒后整页逐字节核对，交回这一页的和。
pub fn check(page: usize, base: usize) -> u64 {
    // SAFETY: 这一页在本 Team 的映射里；这一次读就是本阶段要观察的那一次访存。
    let head = unsafe { core::ptr::read_volatile(base as *const u8) };
    assert_eq!(head, at(page, 0), "memory-transfer: page {page} head byte");
    let mut total = head as u64;
    for offset in 1..PAGE_SIZE {
        // SAFETY: 同上。
        let got = unsafe { core::ptr::read_volatile((base + offset) as *const u8) };
        assert_eq!(
            got,
            at(page, offset),
            "memory-transfer: page {page} byte {offset}"
        );
        total += got as u64;
    }
    total
}
