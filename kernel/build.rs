use std::env as std_env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use env::ProgramKind;
use env::wire::manifest;

/// initrd 承载的引导期程序（**临时机制**，见 kernel/src/initrd.rs）：
/// (清单名, cargo bin 名, 特权级)。顺序无关——**清单解释权在 root 域程序**，
/// 内核只按 `ROOT_NAME` 取引导镜像。
/// 这里是**唯一**声明「程序装成哪种空间」的地方——root 从清单里读，不再硬编码。
/// 码（`ProgramKind` → u32）在 `env::wire::manifest` 里写死一次，本表只用类型。
const ROOT_NAME: &str = "root";
const INITRD_BINS: &[(&str, &str, ProgramKind)] = &[
    (ROOT_NAME, "prog-root", ProgramKind::Supervisor),
    // 调试回显：**U 态**（最小特权）——它只走 `env` 的调试面（`DebugCall`），
    // 够不着建域那道 S 态门。
    ("echo", "prog-echo", ProgramKind::User),
    // 中断面域：**S 态**——它要读写 PLIC 的寄存器（那一页由 root 从配对块取出来授给它，
    // 内核不参与；内核只摇那枚铃）。
    ("plic", "prog-plic", ProgramKind::Supervisor),
];

fn main() {
    // 内核链接脚本：workspace 化后不同 crate 用不同 -Tlink.ld（内核 0x80200000 /
    // 用户 0x10000），不能放根 .cargo/config.toml（全局 rustflags 冲突），改由
    // build.rs 传绝对路径。
    let ld = format!("{}/link.ld", env!("CARGO_MANIFEST_DIR"));
    println!("cargo::rustc-link-arg=-T{ld}");
    println!("cargo::rerun-if-changed=link.ld"); // link.ld 变更自动重链

    // 镜像程序 ELF 打包进 initrd（boot 不再 include_bytes 内嵌）：工作区没有
    // kernel→programs 的依赖边，`cargo clean` 后可能先编 kernel 而程序产物尚不存在
    // → initrd 打包报"文件缺失"。这里在编译前显式构建 programs crate，并从产物
    // 路径读字节写 initrd。
    //
    // 嵌套 cargo 必须用**独立 target 目录**（$OUT_DIR/programs）：宿主 cargo 会在
    // target 根持有 .cargo-build-lock，同目录再起 cargo 会互锁死等。隔离目录无此问题，
    // 且随 cargo clean 一并清除（每次从零重build，无陈旧产物）。
    let target = std_env::var("TARGET").expect("TARGET env missing");
    let bin_target =
        Path::new(&std_env::var("OUT_DIR").expect("OUT_DIR env missing")).join("programs");
    let cargo = std_env::var("CARGO").expect("CARGO env missing");
    let mut args = vec![
        "build".to_string(),
        "-p".to_string(),
        "programs".to_string(),
        "--target".to_string(),
        target.clone(),
        "--target-dir".to_string(),
        bin_target.to_str().expect("non-utf8 OUT_DIR").to_string(),
    ];
    // PROFILE = "debug" 对应 dev profile（cargo 不接受 --profile debug）；其余按名透传
    // （release 等）。内层 cargo 与宿主同 profile，产物目录名一致。
    let profile = std_env::var("PROFILE").expect("PROFILE env missing");
    if profile != "debug" {
        args.push("--profile".to_string());
        args.push(profile.clone());
    }
    let status = Command::new(&cargo)
        .args(&args)
        .status()
        .expect("failed to spawn cargo for programs crate");
    assert!(
        status.success(),
        "programs crate build failed (kernel packs program ELFs into initrd)"
    );

    // 写 initrd 清单（**临时机制**，见 kernel/src/initrd.rs）：与内核 ELF 同目录，
    // runner 从内核 ELF 的父目录取它传给 QEMU `-initrd`。格式的单一真相在
    // `env::wire::manifest`——读它的那一侧是 root 域，**写读共用一批判据**。
    let bin_dir = bin_target.join(&target).join(&profile);
    let main_profile = main_profile_dir(&std_env::var("OUT_DIR").expect("OUT_DIR env missing"));
    let blob_path = main_profile.join("initrd.img");
    let mut images: Vec<(ProgramKind, &str, Vec<u8>)> = Vec::new();
    for (name, bin, kind) in INITRD_BINS {
        let elf = fs::read(bin_dir.join(bin))
            .unwrap_or_else(|e| panic!("initrd: read {bin} from {}: {e}", bin_dir.display()));
        images.push((*kind, name, elf));
    }
    let items: Vec<(ProgramKind, &str, &[u8])> = images
        .iter()
        .map(|(kind, name, elf)| (*kind, *name, elf.as_slice()))
        .collect();
    let (blob, spans) =
        manifest::pack(&items).expect("initrd: 清单越界（条数 / 名字长度 / 空镜像）");
    // 引导镜像在清单内的偏移/长度（内核不解析清单，按这两个常量取 root 的 ELF）
    let at = INITRD_BINS
        .iter()
        .position(|(name, _, _)| *name == ROOT_NAME)
        .expect("initrd: ROOT_NAME not in INITRD_BINS");
    let root_span = spans[at].clone();
    assert!(!root_span.is_empty(), "initrd: root image is empty");
    println!("cargo::rustc-env=ROOT_OFFSET={}", root_span.start);
    println!("cargo::rustc-env=ROOT_LEN={}", root_span.len());
    fs::write(&blob_path, &blob)
        .unwrap_or_else(|e| panic!("initrd: write {}: {e}", blob_path.display()));
    println!(
        "initrd packed: {} ({} B, {} programs)",
        blob_path.display(),
        blob.len(),
        INITRD_BINS.len()
    );
    // 程序侧任何一层变了，initrd 里的程序也得重打包——否则内核重编而镜像是旧的。
    // 这几条曾经漏掉过一次（"改了程序、initrd 还是旧的"），故四层都 watch。
    println!("cargo::rerun-if-changed=../programs");
    println!("cargo::rerun-if-changed=../crates/runtime");
    println!("cargo::rerun-if-changed=../crates/protocol");
    // env 是 runtime/protocol/programs 的路径依赖（envcall 骨架/协议编解码）——它变了
    // initrd 里的程序也得重打包，否则内核重编而用户程序是旧的（曾导致 ebreak 改动未生效）。
    println!("cargo::rerun-if-changed=../crates/env");
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
