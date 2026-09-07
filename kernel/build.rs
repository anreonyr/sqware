use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// initrd 承载的唯一用户程序（shell）。boot 把整个 initrd 当作单个 ELF 装载。
/// 其余 demo 不再装入内核镜像/initrd——保留为用户 crate 的独立 bin，供后续
/// 「字节内嵌 shell + SpawnTeam」机制按需装载。
const INITRD_BIN: &str = "user-shell";

fn main() {
    // 内核链接脚本：workspace 化后不同 crate 用不同 -Tlink.ld（内核 0x80200000 /
    // 用户 0x10000），不能放根 .cargo/config.toml（全局 rustflags 冲突），改由
    // build.rs 传绝对路径。
    let ld = format!("{}/link.ld", env!("CARGO_MANIFEST_DIR"));
    println!("cargo::rustc-link-arg=-T{ld}");
    println!("cargo::rerun-if-changed=link.ld"); // link.ld 变更自动重链

    // 用户程序 ELF 打包进 initrd（boot 不再 include_bytes 内嵌）：工作区没有
    // kernel→user 的依赖边，`cargo clean` 后可能先编 kernel 而 user 产物尚不存在
    // → initrd 打包报"文件缺失"。这里在编译前显式构建 user crate，并从产物
    // 路径读字节写 initrd。
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
        "user crate build failed (kernel packs shell ELF into initrd)"
    );

    // 写 initrd（= shell ELF 原样拷贝，无清单）：与内核 ELF 同目录，runner 从
    // 内核 ELF 的父目录取它传给 QEMU `-initrd`。
    let bin_dir = user_target.join(&target).join(&profile);
    let main_profile = main_profile_dir(&env::var("OUT_DIR").expect("OUT_DIR env missing"));
    let blob_path = main_profile.join("initrd.img");
    let elf_path = bin_dir.join(INITRD_BIN);
    let bytes = fs::read(&elf_path)
        .unwrap_or_else(|e| panic!("initrd: read {INITRD_BIN} from {}: {e}", elf_path.display()));
    fs::write(&blob_path, &bytes)
        .unwrap_or_else(|e| panic!("initrd: write {}: {e}", blob_path.display()));
    println!("initrd packed: {} ({} B)", blob_path.display(), bytes.len());
    println!("cargo::rerun-if-changed=../user");
}

/// 从 OUT_DIR（`.../<profile>/build/<pkg>/<hash>/out`）向上找到 `<profile>` 目录：
/// 它是**唯一**直接包含 `build` 子目录的祖先（内核 ELF 与 runner 读 initrd 均在此）。
fn main_profile_dir(out_dir: &str) -> PathBuf {
    let mut cur: &Path = Path::new(out_dir);
    while !cur.join("build").is_dir() {
        cur = cur.parent().expect("OUT_DIR layout too shallow");
    }
    cur.to_path_buf()
}
