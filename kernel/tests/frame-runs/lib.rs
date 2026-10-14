//! 宿主台架：把 production 的帧分段迭代器（`map/range.rs`、`map/run.rs`、
//! `map/runs.rs`）原样夹进来，只补它碰得到的那点叶子依赖。
//!
//! 夹进来的三份文件是**同一份实现**（`include!`，不是抄一遍），`MapRange`、
//! `PteFlags`、`VirtAddr`/`PhysAddr` 也走 production 那两份源文件。
//!
//! 这台只判分段 / flags / 错误值 / 迭代器零分配：帧是「带物理地址的占位」，
//! 不碰真实物理内存、不碰页表、不碰 Tree/Map 的锁。那些留内核内用例。
//!
//! 跑法（显式 x86_64、离线）：
//!   cargo test --manifest-path kernel/tests/frame-runs/Cargo.toml \
//!     --target x86_64-unknown-linux-gnu --offline
#![allow(dead_code, unused_imports)]

extern crate alloc;

#[path = "support/alloc.rs"]
mod counting_alloc;

mod core {
    pub mod memory {
        pub const PAGE_SIZE: usize = 4096;
        pub const PAGE_SHIFT: usize = 12;

        pub mod manager {
            pub mod addr {
                pub mod phys {
                    include!("../../src/core/memory/manager/addr/phys.rs");
                }
                pub mod virt {
                    include!("../../src/core/memory/manager/addr/virt.rs");
                }

                pub use phys::PhysAddr;
                pub use virt::VirtAddr;
            }
            pub mod entry {
                pub mod flags {
                    include!("../../src/core/memory/manager/entry/flags.rs");
                }
                pub mod pte {
                    include!("../../src/core/memory/manager/entry/pte.rs");
                }

                pub use flags::PteFlags;
                pub use pte::PageTableEntry;
            }
            pub mod frame {
                include!("support/frame.rs");
            }
            pub mod mode {
                include!("support/mode.rs");
            }

            include!("support/error.rs");

            pub mod tree {
                pub mod map {
                    pub mod range {
                        include!("../../src/core/memory/manager/tree/map/range.rs");
                    }
                    pub mod run {
                        include!("../../src/core/memory/manager/tree/map/run.rs");
                    }
                    pub mod runs {
                        include!("../../src/core/memory/manager/tree/map/runs.rs");
                    }
                    #[cfg(test)]
                    pub mod tests {
                        include!("tests.rs");
                    }
                }
            }
        }
    }
}
