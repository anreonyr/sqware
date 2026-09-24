//! image — **造镜像那一步**：编程序（`programs` + `harness`）→ 打 initrd → 放到内核 ELF 旁。
//!
//! # 照实记（它为什么不在 `kernel/build.rs` 里）
//!
//! 它原先在那儿，那让内核的**编译单元**背上了三件不属于它的事：
//!
//!   ① 编全部程序（嵌套 cargo）；② 打 initrd；③ 把引导镜像的偏移/长度**回喂**内核源码
//!   （`cargo::rustc-env`）。
//!
//! 用户原话：**"initrd 与 kernel 何干"**。后果是实测出来的：`watch()` 必须把 programs /
//! harness / crates 的**每一个文件**登记成 `rerun-if-changed`（否则"改了程序、行为不变"），
//! 于是**改一个客人的一个字节，内核 crate 就重编**（实测 1.31 s/次），`cargo build` 也永远是
//! "编内核 + 编全部程序 + 打包"。
//!
//! **照实记（`watch()` 那套为什么不必跟过来）**：它存在是因为打包**寄生在 build script 里**
//! ——build script 只由 cargo 的指纹机制驱动，"镜像新鲜不新鲜"没有任何人负责，只好把源文件
//! 全盯上。搬出来之后**谁造镜像谁负责新鲜**：门每一轮都造（`crates/gate`），交互式由
//! `scripts/runner.nu` 先查在不在。那条"逐文件盯"的纪律连同它挡过的两类假象一起消失。
//!
//! 造出来的东西与内核**只共享一个约定**：initrd 落在内核 ELF **同目录**（`boot.nu` 就在那儿找）。
//! 内核也不再需要那两个数——它们写在区里前 8 字节（`env::wire::manifest::PREAMBLE`），开机读。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 目标三元组（与根 `.cargo/config.toml` 钉的那个一致）。
const TARGET: &str = "riscv64gc-unknown-none-elf";

/// 仓库根（本 crate 在 `crates/image`）。
fn root() -> PathBuf {
    let at = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    at.canonicalize().unwrap_or(at)
}

/// 认得的场景名——**从引导镜像那张表里收**（一个景存在 ⇔ 它有一条引导镜像），故不会与它脱节。
fn scenes() -> Vec<&'static str> {
    env::assembly::ENTRY.iter().map(|(s, _)| *s).collect()
}

/// 这一景要装的程序（**装配单按 `scenes` 过滤**；次序即装载次序）。
fn bins_for(scenario: &str) -> Result<Vec<(&'static str, env::ProgramKind)>, String> {
    let picked: Vec<(&'static str, env::ProgramKind)> = env::assembly::ALL
        .iter()
        .filter(|row| row.scenes.contains(&scenario))
        .map(|row| (row.name, row.kind))
        .collect();
    if picked.is_empty() {
        return Err(format!(
            "不认得的景 {scenario}（认得的：{}）",
            scenes().join(" / ")
        ));
    }
    Ok(picked)
}

/// 造这一景这一档的镜像，返 `initrd.img` 的落点（内核 ELF 同目录）。
pub fn build(scenario: &str, profile: &str) -> Result<PathBuf, String> {
    let root = root();
    let bins = bins_for(scenario)?;

    // 嵌套 cargo 用**独立 target 目录**：宿主 cargo 会在 target 根持有 `.cargo-build-lock`，
    // 同目录再起 cargo 会互锁死等（照实记：这一格是从 `kernel/build.rs` 原样搬过来的）。
    let work = root.join("target/image").join(profile);
    let mut args = vec![
        "build".to_string(),
        // **两个包一起编**：产品（`programs`）与测具（`harness`）——后者依赖前者的 lib。
        "-p".to_string(),
        "programs".to_string(),
        "-p".to_string(),
        "harness".to_string(),
        "--target".to_string(),
        TARGET.to_string(),
        "--target-dir".to_string(),
        work.to_string_lossy().into_owned(),
    ];
    // `debug` 是 dev profile 的名字——cargo 不接受 `--profile debug`，其余按名透传。
    if profile != "debug" {
        args.push("--profile".to_string());
        args.push(profile.to_string());
    }
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    // 读数开关：**由调用方的环境决定**（`env::readings` 的 `option_env!`）。门要读数 ⇒
    // 它在自己的进程里带着 `SQWARE_READINGS=1`（见 `crates/gate/src/lib.rs`）；`cargo image`
    // 这条人手走的路默认不带 ⇒ 产品那一颗从编出来就是静默的。
    let readings = std::env::var("SQWARE_READINGS").is_ok();
    let status = Command::new(&cargo)
        .args(&args)
        .env("SQWARE_READINGS", if readings { "1" } else { "" })
        .current_dir(&root)
        .status()
        .map_err(|e| format!("起不动嵌套 cargo：{e}"))?;
    if !status.success() {
        return Err("programs / harness 编不过（上面是它们自己的报错）".to_string());
    }

    // 读产物：**bin 名是约定** `prog-<名字>`（装配单里不写第二遍）。
    let bin_dir = work.join(TARGET).join(profile);
    let mut images = Vec::with_capacity(bins.len());
    for (name, kind) in &bins {
        let bin = format!("prog-{name}");
        let elf =
            std::fs::read(bin_dir.join(&bin)).map_err(|e| format!("读 {bin} 失败：{e}"))?;
        images.push((*kind, *name, elf));
    }
    let items: Vec<(env::ProgramKind, &str, &[u8])> = images
        .iter()
        .map(|(kind, name, elf)| (*kind, *name, elf.as_slice()))
        .collect();
    // 引导镜像**按名字查**（不是"跟景同名"）：`product` 那一景的引导镜像仍是 `root`
    // （见 `env::assembly::ENTRY` 那条照实记）。它必须在清单里——不在就是装配单写错了。
    let entry = env::assembly::entry_of(scenario)
        .ok_or_else(|| format!("initrd: 不认得的景 {scenario}（认得的：{}）", scenes().join(" / ")))?;
    let root_at = bins
        .iter()
        .position(|(name, _)| *name == entry)
        .ok_or_else(|| format!("initrd: 景 {scenario} 的引导镜像 {entry} 不在这一景的清单里"))?;
    let blob = env::wire::manifest::pack(&items, root_at)
        .ok_or_else(|| "initrd: 清单越界（条数 / 名字长度 / 空镜像）".to_string())?;

    // 落点：内核 ELF 同目录（`boot.nu` 就在那儿找）。
    let at = root
        .join("target")
        .join(TARGET)
        .join(profile)
        .join("initrd.img");
    if let Some(dir) = at.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("建 {} 失败：{e}", dir.display()))?;
    }
    std::fs::write(&at, &blob).map_err(|e| format!("写 {} 失败：{e}", at.display()))?;
    println!(
        "initrd packed: {} ({} B, {} programs, 档 {profile})",
        at.display(),
        blob.len(),
        bins.len()
    );
    // **档只有一处说话，那就得有人替它兜底**：这一景要的核内 ELF 与 initrd 是**同目录的
    // 兄弟**（`boot.nu` / `runner.nu` 都按"内核同目录"推导镜像）。少了它、或它比 initrd 旧，
    // 症状是"起得来、但跑的不是刚打的那一颗"——**不报错的错**。故这里当场说破。
    guard_kernel_sibling(&at, profile)?;
    Ok(at)
}

/// 核内 ELF 必须已经在 initrd 旁边，而且**不比 initrd 旧**。
///
/// 为什么只报不改：编内核那一半归 `cargo build`（"initrd 与 kernel 何干"那条裁定），
/// 这一层不越界替它编；但"你指定的那一个档"这句话里**包含**那另一半，故由它来说破。
fn guard_kernel_sibling(initrd: &std::path::Path, profile: &str) -> Result<(), String> {
    let elf = initrd.with_file_name("sqware");
    let build = if profile == "debug" {
        "cargo build".to_string()
    } else {
        format!("cargo build --profile {profile}")
    };
    let meta = std::fs::metadata(&elf).map_err(|_| {
        format!(
            "initrd 落好了，但同目录没有内核 ELF：{}\n  先编内核：{build}",
            elf.display()
        )
    })?;
    let initrd_at = std::fs::metadata(initrd)
        .and_then(|m| m.modified())
        .map_err(|e| format!("读 initrd 时间戳失败：{e}"))?;
    let elf_at = meta
        .modified()
        .map_err(|e| format!("读内核时间戳失败：{e}"))?;
    if elf_at < initrd_at {
        return Err(format!(
            "内核 ELF 比 initrd 旧（档 {profile}）：{}\n  重新编内核：{build}",
            elf.display()
        ));
    }
    Ok(())
}
