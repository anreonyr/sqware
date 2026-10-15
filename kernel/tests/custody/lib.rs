//! 宿主台架：把 production 的 `flat/custody.rs` 原样夹进来，只补它碰得到的那点叶子依赖。
//!
//! 夹进来的那一份是**同一份实现**（`include!`，不是抄一遍）：真 `Custody` 与它真正的批量登记
//! `register`。这台只判状态机本身（单取独占 / 批量登记 / 放回 / 幂等），
//! 不碰帧、不碰 Flat、不碰锁。
//!
//! 跑法（显式 x86_64、离线）：
//!   cargo test --manifest-path kernel/tests/custody/Cargo.toml \
//!     --target x86_64-unknown-linux-gnu --offline
#![cfg(test)]
#![allow(dead_code, unused_imports)]

mod flat {
    pub mod custody {
        include!("../../src/core/memory/flat/custody.rs");
    }
    pub mod error {
        include!("fixture/error.rs");
    }

    #[cfg(test)]
    mod tests {
        include!("tests.rs");
    }
}
