//! 宿主台架：把 production 的帧分段迭代器（`nest/view/active.rs`、`run.rs`、
//! `runs.rs`）原样夹进来，只补它碰得到的那点叶子依赖。
//!
//! 夹进来的三份文件是**同一份实现**（`include!`，不是抄一遍），`ActiveRange`、
//! `Access`、`PteFlags`、`VirtAddr`/`PhysAddr` 也走 production 那几份源文件。
//!
//! 这台只判分段 / 权限 / 错误值 / 迭代器零分配：帧是「带物理地址的占位」，
//! 不碰真实物理内存、不碰页表、不碰 Nest/View 的锁。那內留内核内用例。
//!
//! 跑法（显式 x86_64、离线）：
//!   cargo test --manifest-path kernel/tests/frame-runs/Cargo.toml \
//!     --target x86_64-unknown-linux-gnu --offline
#![allow(dead_code, unused_imports)]

extern crate alloc;

#[path = "support/alloc.rs"]
mod counting_alloc;

mod machine {
    pub mod mode {
        include!("support/mode.rs");
    }
}

mod core {
    pub mod memory {
        pub const PAGE_SIZE: usize = 4096;
        pub const PAGE_SHIFT: usize = 12;

        pub mod address {
            pub mod phys {
                include!("../../src/core/memory/address/phys.rs");
            }
            pub mod virt {
                include!("../../src/core/memory/address/virt.rs");
            }

            pub use phys::PhysAddr;
            pub use virt::VirtAddr;
        }
        pub mod flat {
            pub mod frame {
                include!("support/frame.rs");
            }

            pub use frame::Frame;
        }

        pub mod nest {
            include!("support/nest_error.rs");

            pub mod table {
                pub mod flags {
                    include!("../../src/core/memory/nest/table/flags.rs");
                }
                pub mod pte {
                    include!("../../src/core/memory/nest/table/entry.rs");
                }

                pub use flags::PteFlags;
                pub use pte::PageTableEntry;
            }

            pub mod view {
                include!("../../src/core/memory/nest/view/access.rs");

                pub mod active {
                    include!("../../src/core/memory/nest/view/active.rs");
                }
                pub mod run {
                    include!("../../src/core/memory/nest/view/run.rs");
                }
                pub mod runs {
                    include!("../../src/core/memory/nest/view/runs.rs");
                }
                #[cfg(test)]
                pub mod tests {
                    include!("tests.rs");
                }

                pub use active::ActiveRange;
            }
        }
    }
}
