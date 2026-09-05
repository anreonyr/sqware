#![no_std]
#![no_main]

extern crate alloc;

use core::ptr;

use ubi::Permission;
use user::env::{io::put, mail::HolePie, mail::PolePie};

// restrict: 收窄本 pie 权限（就地改写，单调）。
//
//   Hole: open → (R|W|VEST|BACK)
//     1. restrict(R)    → Ok（收窄到只读）
//     2. push           → Err（无 W）（F1）
//     3. restrict(R|W)  → Err（非单调：收窄后不得再放宽）（F2）
//
//   Pole: open(PAGE) → (R|W|VEST|BACK)，auto-map R|W
//     1. restrict(R)   → Ok（收窄到只读 + 映射段降权）
//     2. map()         → Ok(VA)，页表已降 R（F3 无 W 语义靠 cap ⊆ 页表）
//
// 共享同一 hole（test 后复用它测 Pole？不，分开开独立 pie，避免相互干扰）。
// Hole 与 Pole 各测各自的 restrict 语义。

const HOLE_MSG_LEN: usize = 64;
const PAGE: usize = 4096;

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("restrict\n");

    // ── Hole: 任意非空子集 ──
    let hole = HolePie::open().expect("open hole");
    // 1. restrict 到只读（R ⊂ R|W|VEST|BACK）→ Ok
    if hole.restrict(Permission::READ).is_ok() {
        let _ = put("H1\n"); // 预期：收窄成功
    } else {
        let _ = put("Hx\n");
    }

    // 2. 收窄后 push（需 W）→ Err（无 W）
    let mut msg = [0u8; HOLE_MSG_LEN];
    msg[0] = 0xAA;
    if hole.push(&msg).is_err() {
        let _ = put("F1\n"); // 预期：Denied
    } else {
        let _ = put("B1\n");
    }

    // 3. 单调违例：restrict 到 R|W（已收窄到 R，R|W ⊄ R）→ Err（Denied）
    if hole.restrict(Permission::READ | Permission::WRITE).is_err() {
        let _ = put("F2\n"); // 预期：Denied（非单调）
    } else {
        let _ = put("B2\n");
    }

    let _ = hole.shut();

    // ── Pole: 须含 READ，restrict 后映射段降权 ──
    let pole = PolePie::open(PAGE).expect("open pole");
    // 1. restrict 到只读（R ⊂ R|W|VEST|BACK）→ Ok
    if pole.restrict(Permission::READ).is_ok() {
        let _ = put("P1\n"); // 预期：收窄成功
    } else {
        let _ = put("Px\n");
    }

    // 2. restrict 到无 READ（R: W，无 READ）→ Err（RISC-V PTE 无 R=0 合法数据叶子）
    if pole.restrict(Permission::WRITE).is_err() {
        let _ = put("F3\n"); // 预期：Denied（Pole 须含 READ）
    } else {
        let _ = put("B3\n");
    }

    // 3. 收窄后 map：R → 返旧 VA（映射段已降权为 R，cap ⊆ 页表）
    match pole.map() {
        Ok(va) => {
            // 只读映射：写会触发页故障（cap ⊆ 页表在此保证只读）。这里只验 map 成功。
            let _ = va;
            let _ = put("M1\n");
        }
        Err(_) => {
            let _ = put("B4\n");
        }
    }

    let _ = pole.shut();
    let _ = put("restrict: done\n");
}
