//! 内核的构建脚本 —— **只剩一件事**：把链接脚本交给 rustc。
//!
//! # 照实记（这里原先干什么，为什么搬走了）
//!
//! 这里原先是**一件**事：① `link-arg`；② `watch()` 把 programs / harness / crates 的
//! **每一个文件**登记成 `rerun-if-changed`；③ 嵌套 cargo 编那两个包；④ `manifest::pack`
//! 打 initrd，并把引导镜像的偏移/长度经 `cargo::rustc-env` **回喂**内核源码。
//!
//! 用户原话：**"initrd 与 kernel 何干"**。②③④ 现在住 **`crates/image`**，由门
//! （`crates/gate`）与 `scripts/runner.nu` 在**编内核之前**调用；内核与它**只剩一个约定**：
//! initrd 落在内核 ELF 同目录。
//!
//! 于是这一份不再需要：
//!
//!   - **`rerun-if-env-changed=SQWARE_ROOT`** —— 换场景不再让内核重编（换的是镜像，不是内核）；
//!   - **`watch()`** —— 那条"逐文件盯"的纪律随打包一起搬去了 `crates/image`（`watch()` 之所以
//!     存在，是因为打包寄生的 build script 只由 cargo 的指纹驱动，"镜像新鲜不新鲜"没人负责；
//!     搬出来之后**谁造镜像谁负责新鲜**）；
//!   - **那两个常量** —— 引导镜像的偏移/长度写在 initrd 区里前 8 字节，内核**开机读**
//!     （`plan::manifest::PREAMBLE`）。内核仍然不认识清单格式：它只读这 8 字节。
//!
//! # 照实记（两条与场景旗标有关的坑，留着，今天没有用户）
//!
//! 从前 `SQWARE_ROOT=fair` 那一景要在这里给程序侧追加一个 `--cfg`，踩出两条规矩：
//!
//!   1. **不能用 `option_env!("SQWARE_ROOT")`** —— cargo 不把 `env!` 读的变量算进指纹
//!      （实测：换变量后引导镜像没换）。旗标得走 `rustflags` 那一族，它**进指纹**。
//!   2. **要写 `CARGO_ENCODED_RUSTFLAGS`，不是 `RUSTFLAGS`** —— 前者优先级更高，只设后者会被
//!      整个盖掉；而编码变量里**已经带着** `.cargo/config.toml` 给本目标配的那几个 `-C…`
//!      旗标，要追加就追加在它后面。分隔符是 `\x1f`。
//!
//! 这两条与场景无关，故留在这里——但**今天这一份一个字都不用它们**。

fn main() {
    // 内核链接脚本：workspace 化后不同 crate 用不同 `-Tlink.ld`（内核 `0x80200000` /
    // 用户 `0x10000`），不能放根 `.cargo/config.toml`（全局 rustflags 冲突），改由本脚本传绝对路径。
    let ld = format!("{}/link.ld", env!("CARGO_MANIFEST_DIR"));
    println!("cargo::rustc-link-arg=-T{ld}");
    println!("cargo::rerun-if-changed=link.ld"); // link.ld 变更自动重链
}
