use std::env;
use std::path::Path;
use std::process::Command;

fn main() {
    // 内核链接脚本：workspace 化后不同 crate 用不同 -Tlink.ld（内核 0x80200000 /
    // 用户 0x10000），不能放根 .cargo/config.toml（全局 rustflags 冲突），改由
    // build.rs 传绝对路径。
    let ld = format!("{}/link.ld", env!("CARGO_MANIFEST_DIR"));
    println!("cargo::rustc-link-arg=-T{ld}");
    println!("cargo::rerun-if-changed=link.ld"); // link.ld 变更自动重链

    // 用户程序 ELF 嵌入（boot::spawn_demos 的 include_bytes!）：工作区没有
    // kernel→user 的依赖边，`cargo clean` 后可能先编 kernel 而 user 产物尚不存在
    // → include_bytes 报"文件缺失"。这里在编译前显式构建 user crate，并把产物
    // 路径经 cargo:rustc-env 暴露给 include_bytes!(env!(...))（boot.rs 消费）。
    //
    // 嵌套 cargo 必须用**独立 target 目录**（$OUT_DIR/user）：宿主 cargo 会在
    // target 根持有 .cargo-build-lock，同目录再起 cargo 会互锁死等。隔离目录无此问题，
    // 且随 cargo clean 一并清除（每次从零重build，无陈旧产物）。
    let target = env::var("TARGET").expect("TARGET env missing");
    let user_target = Path::new(&env::var("OUT_DIR").expect("OUT_DIR env missing")).join("user");
    let cargo = env::var("CARGO").expect("CARGO env missing");
    let mut args = vec![
        "build".to_string(),
        "-p".to_string(),
        "user".to_string(),
        "--target".to_string(),
        target.clone(),
        "--target-dir".to_string(),
        user_target.to_str().expect("non-utf8 OUT_DIR").to_string(),
    ];
    // PROFILE = "debug" 对应 dev profile（cargo 不接受 --profile debug）；其余按名透传
    // （release 等）。内层 cargo 与宿主同 profile，产物目录名一致。
    let profile = env::var("PROFILE").expect("PROFILE env missing");
    if profile != "debug" {
        args.push("--profile".to_string());
        args.push(profile.clone());
    }
    let status = Command::new(&cargo)
        .args(&args)
        .status()
        .expect("failed to spawn cargo for user crate");
    assert!(
        status.success(),
        "user crate build failed (kernel embeds its ELFs via include_bytes!)"
    );

    // 暴露各用户二进制绝对路径：boot.rs include_bytes!(env!(...)) 使用。
    // 顺带消除 boot.rs 里硬编码 "/debug/" 的脆弱点（release 构建同样可用）。
    let bin_dir = user_target.join(&target).join(&profile);
    println!(
        "cargo::rustc-env=USER_HEAPER={}",
        bin_dir.join("user-heaper").display()
    );
    println!(
        "cargo::rustc-env=USER_SPAWNER={}",
        bin_dir.join("user-spawner").display()
    );
    println!(
        "cargo::rustc-env=USER_MMAPER={}",
        bin_dir.join("user-mmaper").display()
    );
    println!(
        "cargo::rustc-env=USER_PORTER={}",
        bin_dir.join("user-porter").display()
    );
    println!(
        "cargo::rustc-env=USER_STRESSOR={}",
        bin_dir.join("user-stressor").display()
    );
    println!(
        "cargo::rustc-env=USER_YIELDER={}",
        bin_dir.join("user-yielder").display()
    );
    println!(
        "cargo::rustc-env=USER_SLEEPER={}",
        bin_dir.join("user-sleeper").display()
    );
    println!(
        "cargo::rustc-env=USER_EXITER={}",
        bin_dir.join("user-exiter").display()
    );
    println!(
        "cargo::rustc-env=USER_TLSER={}",
        bin_dir.join("user-tlser").display()
    );
    println!(
        "cargo::rustc-env=USER_DOCKER={}",
        bin_dir.join("user-docker").display()
    );
    println!(
        "cargo::rustc-env=USER_RINGER={}",
        bin_dir.join("user-ringer").display()
    );
    println!(
        "cargo::rustc-env=USER_LISP={}",
        bin_dir.join("user-lisp").display()
    );
    // 用户源码/清单变更 → 重跑本脚本（重建 user + 重编内核）
    println!("cargo::rerun-if-changed=../user");
}
