#![no_std]
#![no_main]

extern crate alloc;

use env::Permission;
use task::env::{io::put, mail::HolePie, mail::PolePie};

// narrow: 收窄本 pie 权限（就地改写，单调）。
//
//   Hole: unseal → (R|W|VEST|BACK)
//     1. narrow(R)    → Ok（收窄到只读）
//     2. push         → Err（无 W）（F1）
//     3. narrow(R|W)  → Err（非单调：收窄后不得再放宽）（F2）
//
//   Pole: unseal(PAGE) → (R|W|VEST|BACK)，auto-map R|W
//     1. narrow(R)   → Ok（收窄到只读 + 映射段降权）
//     2. narrow(W)   → Err（无 READ：RISC-V PTE 无 R=0 合法数据叶子）（F3）
//     3. map()       → Ok(VA)，页表已降 R（cap ⊆ 页表）

const HOLE_MSG_LEN: usize = 64;
const PAGE: usize = 4096;

#[unsafe(no_mangle)]
extern "C" fn main() {
    let _ = put("narrow\n");

    // ── Hole: 任意非空子集 ──
    let hole = HolePie::unseal().expect("unseal hole");
    // 1. narrow 到只读（R ⊂ R|W|VEST|BACK）→ Ok
    if hole.narrow(Permission::READ).is_ok() {
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

    // 3. 单调违例：narrow 到 R|W（已收窄到 R，R|W ⊄ R）→ Err（Denied）
    if hole.narrow(Permission::READ | Permission::WRITE).is_err() {
        let _ = put("F2\n"); // 预期：Denied（非单调）
    } else {
        let _ = put("B2\n");
    }

    let _ = hole.seal();

    // ── Pole: 须含 READ，narrow 后映射段降权 ──
    let pole = PolePie::unseal(PAGE).expect("unseal pole");
    // 1. narrow 到只读（R ⊂ R|W|VEST|BACK）→ Ok
    if pole.narrow(Permission::READ).is_ok() {
        let _ = put("P1\n"); // 预期：收窄成功
    } else {
        let _ = put("Px\n");
    }

    // 2. narrow 到无 READ（仅 W）→ Err（RISC-V PTE 无 R=0 合法数据叶子）
    if pole.narrow(Permission::WRITE).is_err() {
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

    let _ = pole.seal();
    let _ = put("narrow: done\n");
}
