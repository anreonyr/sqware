//! programs 的构建脚本：只做一件事——把镜像链接脚本交给 rustc。

fn main() {
    // 镜像程序固定链接在 IMAGE_BASE (0x10000)，见 link.ld。
    // 本 crate 的每个 [[bin]] 都由内核 `build.rs` 经嵌套 cargo 构建并打进 initrd。
    let ld = format!("{}/link.ld", env!("CARGO_MANIFEST_DIR"));
    println!("cargo::rustc-link-arg=-T{ld}");
    println!("cargo::rerun-if-changed=link.ld");

    // **每个 bin 的源都要盯**：本 crate 的 `[[bin]]` 那些文件**不在** Cargo 的默认重编扫描里
    // （默认只盯 `src/lib.rs` 那一族），源码改了它可能不重编 ⇒ 下游（`crates/image`、门）
    // 拿到的是**旧产物**。实测栽过：`cargo image` 打出旧 initrd，量出来的东西其实不是刚改的。
    // 一条一条列（不走 `src` 目录的 `rerun-if-changed`：那是未定义行为），让 cargo 自己算指纹。
    // **本表与 `Cargo.toml` 的 `[[bin]]` 一一对应**（34 条：产品 9 ＋ 测具 25）——改一处要同时改两处。
    println!("cargo::rerun-if-changed=src/lib.rs");
    println!("cargo::rerun-if-changed=src/user/canonical/main.rs");
    println!("cargo::rerun-if-changed=src/driver/router/main.rs");
    println!("cargo::rerun-if-changed=src/driver/uart/main.rs");
    println!("cargo::rerun-if-changed=src/driver/rtc/main.rs");
    println!("cargo::rerun-if-changed=src/system/main.rs");
    println!("cargo::rerun-if-changed=src/service/operator/main.rs");
    println!("cargo::rerun-if-changed=src/service/principal/main.rs");
    println!("cargo::rerun-if-changed=src/service/coalition/main.rs");
    println!("cargo::rerun-if-changed=src/service/hub/main.rs");
    println!("cargo::rerun-if-changed=src/harness/probe/probe_lease.rs");
    println!("cargo::rerun-if-changed=src/harness/probe/probe_owner.rs");
    println!("cargo::rerun-if-changed=src/harness/probe/probe_denied.rs");
    println!("cargo::rerun-if-changed=src/harness/probe/probe_bound.rs");
    println!("cargo::rerun-if-changed=src/harness/probe/probe_control.rs");
    println!("cargo::rerun-if-changed=src/harness/probe/probe_operator_gate.rs");
    println!("cargo::rerun-if-changed=src/harness/probe/probe_operator_land.rs");
    println!("cargo::rerun-if-changed=src/harness/probe/probe_rule.rs");
    println!("cargo::rerun-if-changed=src/harness/probe/probe_rule_other.rs");
    println!("cargo::rerun-if-changed=src/harness/guest/guest.rs");
    println!("cargo::rerun-if-changed=src/harness/guest/passer.rs");
    println!("cargo::rerun-if-changed=src/harness/guest/lodger.rs");
    println!("cargo::rerun-if-changed=src/harness/guest/sleeper.rs");
    println!("cargo::rerun-if-changed=src/harness/guest/subject.rs");
    println!("cargo::rerun-if-changed=src/harness/guest/member.rs");
    println!("cargo::rerun-if-changed=src/harness/bench/again/churn.rs");
    println!("cargo::rerun-if-changed=src/harness/bench/rig/rig.rs");
    println!("cargo::rerun-if-changed=src/harness/bench/load/busy.rs");
    println!("cargo::rerun-if-changed=src/harness/bench/load/park.rs");
    println!("cargo::rerun-if-changed=src/harness/bench/rig/hang.rs");
    println!("cargo::rerun-if-changed=src/harness/bench/load/load.rs");
    println!("cargo::rerun-if-changed=src/harness/bench/beat.rs");
    println!("cargo::rerun-if-changed=src/harness/bench/again/again.rs");
    println!("cargo::rerun-if-changed=src/harness/bench/group/waiter.rs");
    println!("cargo::rerun-if-changed=src/harness/bench/group/group.rs");
}
