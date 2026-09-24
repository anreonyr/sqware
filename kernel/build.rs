use std::env as std_env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use env::ProgramKind;
use env::wire::manifest;

/// initrd 承载的引导期程序（**临时机制**，见 `kernel/src/platform/initrd.rs`）：
/// (清单名, 特权级)——**bin 名不写**（它是约定 `prog-<名字>`，见下面那条照实记）。
/// 顺序无关——**清单解释权在 root 域程序**，
/// 内核只按 `SQWARE_ROOT`（本脚本运行时读）选出的 `ROOT_OFFSET`/`ROOT_LEN` 取引导镜像。
/// 这里是**唯一**声明「程序装成哪种空间」的地方——root 从清单里读，不再硬编码。
/// 码（`ProgramKind` → u32）在 `env::wire::manifest` 里写死一次，本表只用类型。
/// 引导镜像的**清单名**：默认 `root`；压测台用 `SQWARE_ROOT=rig` 换一个（见
/// `harness/src/rig.rs`）——换的只是"谁的镜像被当引导镜像"，内核其余一字不改。
///
/// **在 build 脚本里按运行时读**（不是 `option_env!`）：`option_env!` 会把值烘进这台
/// 脚本自己的二进制，而脚本何时重编不由那个变量决定（实测换变量后引导镜像没换）；
/// 运行时读 + `rerun-if-env-changed` 才是 build 脚本该用的那一对。
fn root_name() -> String {
    std::env::var("SQWARE_ROOT").unwrap_or_else(|_| "root".to_string())
}

/// **装配单在 `env::assembly`**——一处声明：名字 · 特权级 · 是什么 · 进哪几张镜像 · 装配参数。
///
/// **照实记（为什么搬走了）**：这里原先有**三张表**（`PRODUCTS` / `PROBES` / `RIGS`）＋`pick`，
/// 而 `programs/src/supervisor/system/scenario.rs` 里另有一份**同名的** `Program` 行与 `PLAN`
/// ——同一条事实写了两处（名字、特权级、在不在表里）。用户原话：**"我不想每次加一个 bin 就写
/// 一个装配表"**。现在两侧读**同一张表**：本脚本（**宿主**）读它决定"哪几台进哪张镜像"，
/// 编排域（**riscv**）读它决定"起谁、什么次序"——而仓里只有 `env` 两边都编得过（见那一处的头注）。
///
/// **次序是硬事实**：`ALL` 的次序就是装载次序（`ROOT_OFFSET` 按位次算），各景按 `scenes` 过滤
/// ⇒ 过滤出来的次序与原先那三张表的次序**逐字一致**（照实记：换表之后 `initrd.img` **逐字节未变**）。
///
/// **bin 名不写**：它是约定 `prog-<名字>`（30 行逐行对过，**零反例**）；改约定要改的是读它的
/// 那一处（下面 `prog-{name}`）与各 crate 的 `[[bin]]`。
fn bins_for(scenario: &str) -> Vec<(&'static str, ProgramKind)> {
    let picked: Vec<(&'static str, ProgramKind)> = env::assembly::ALL
        .iter()
        .filter(|row| row.scenes.contains(&scenario))
        .map(|row| (row.name, row.kind))
        .collect();
    assert!(
        !picked.is_empty(),
        "未知场景 SQWARE_ROOT={scenario}（认得的：{}）",
        scenes().join(" / ")
    );
    picked
}

/// 认得的场景名——**从装配单里收**，故不会与它脱节（原先是 panic 消息里手写的一串）。
fn scenes() -> Vec<&'static str> {
    let mut all: Vec<&'static str> = Vec::new();
    for row in env::assembly::ALL {
        for scene in row.scenes {
            if !all.contains(scene) {
                all.push(scene);
            }
        }
    }
    all
}

fn main() {
    // 内核链接脚本：workspace 化后不同 crate 用不同 -Tlink.ld（内核 0x80200000 /
    // 用户 0x10000），不能放根 .cargo/config.toml（全局 rustflags 冲突），改由
    // build.rs 传绝对路径。
    let ld = format!("{}/link.ld", env!("CARGO_MANIFEST_DIR"));
    println!("cargo::rustc-link-arg=-T{ld}");
    println!("cargo::rerun-if-changed=link.ld"); // link.ld 变更自动重链
    // 换引导镜像（压测台）只改这一个环境变量：让 cargo 在它变时重跑本脚本。
    println!("cargo::rerun-if-env-changed=SQWARE_ROOT");

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
        // **两个包一起编**：产品（`programs`）与测具（`harness`）——后者依赖前者的 lib。
        "-p".to_string(),
        "programs".to_string(),
        "-p".to_string(),
        "harness".to_string(),
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
    // **照实记（场景旗标这一格今天没有用户，但那两条坑留着）**：`SQWARE_ROOT=fair` 还在的时候，
    // 这里给程序侧追加过一个 `--cfg sqware_fair`（装配单按它选表）。下面两条规矩与场景无关：
    //
    // 照实记（两处坑，都是实测踩出来的）：
    //
    //  1) **不能用 `option_env!("SQWARE_ROOT")`**——cargo 不把 `env!` 读的变量算进指纹
    //     （本文件头注记着同一个坑：*"脚本何时重编不由那个变量决定（实测换变量后引导镜像没换）"*）。
    //     旗标得走 `rustflags` 那一族：它**进指纹**，换场景必然重编。
    //  2) **要写 `CARGO_ENCODED_RUSTFLAGS`，不是 `RUSTFLAGS`**——cargo 给 build 脚本的那个编码
    //     变量**优先级更高**：只设 `RUSTFLAGS` 会被它整个盖掉（实测：那版跑出来聊天客人一次
    //     都没出现）。而且编码变量里**已经带着** `.cargo/config.toml` 给本目标配的那几个
    //     `-C…` 旗标（`relocation-model` / `frame-pointers` / `code-model`）——要追加就追加在
    //     它后面，那几个才不会被顶掉。分隔符是 `\x1f`。
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
    let bins = bins_for(&root_name());
    for (name, kind) in &bins {
        // **bin 名是约定**（`prog-<名字>`）——表里不再写第二遍，见那三张表的照实记。
        let bin = format!("prog-{name}");
        let elf = fs::read(bin_dir.join(&bin))
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
    let at = bins
        .iter()
        .position(|(name, _)| *name == root_name())
        .unwrap_or_else(|| panic!("initrd: SQWARE_ROOT={} 不在这一景的清单里", root_name()));
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
        bins.len()
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
    // **照实记（这一行是漏出来的）**：`harness`（测具那一档）搬出去之后**没被盯上**——
    // "改了程序、行为不变"那一类假象当场又露了一次头：探测那台的 `[case]` 协议一行都不出现，
    // 因为 initrd 里装的是**上一版**探针。搬家的那一刀漏的就是这一行。
    for tree in ["programs", "harness", "crates"] {
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
