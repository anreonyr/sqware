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
    // 客人：**U 态**（与 `echo` 同一档）——按名字找到一个服务、说一句话。铸孔、交出、
    // 一问一答都不需要 S 态，故最小特权的域也能用板。
    ("guest", "prog-guest", ProgramKind::User),
    // 过客：**U 态**（同上）——起来、挂一个名字、**直接死**（不说再见）。它与 `guest` 只差
    // 少说那一句退场：板那两条判据里"这位还在吗"（`Alive`）那一格靠它做读数。
    ("passer", "prog-passer", ProgramKind::User),
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
    //
    // **逐文件**，不是逐目录：`rerun-if-changed=<目录>` 只在**目录条目增删**时触发，
    // 原地改一个文件**不触发**（cargo 对目录只比 mtime，而改文件不改目录的 mtime）。
    // 这几条曾经漏掉过一次，之后改成 watch 目录——但仍然漏掉"原地改文件"，症状是
    // "改了程序、行为不变"，最费时间的一类假象。逐文件列全之后这一格不再存在。
    watch(Path::new(".."));
}

/// 逐文件登记"本脚本要盯的东西"：workspace 的源码与清单。
///
/// 走 `../programs`、`../crates` 两棵子树（跳过 `target`）＋工作区清单（`Cargo.toml`
/// / `Cargo.lock`：依赖版本变了同样要重打包）。**只盯文件，不盯目录**——理由见调用处。
fn watch(root: &Path) {
    for tree in ["programs", "crates"] {
        walk(&root.join(tree));
    }
    for manifest in ["Cargo.toml", "Cargo.lock"] {
        let at = root.join(manifest);
        if at.is_file() {
            println!("cargo::rerun-if-changed={}", at.display());
        }
    }
}

/// 递归登记一棵子树里**每一个文件**。
fn walk(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let at = entry.path();
        let name = entry.file_name();
        // 产物目录与隐藏目录不进：它们不是"源"，盯它们只会白白重跑。
        if name == "target" || name.to_string_lossy().starts_with('.') {
            continue;
        }
        if at.is_dir() {
            walk(&at);
        } else {
            println!("cargo::rerun-if-changed={}", at.display());
        }
    }
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
