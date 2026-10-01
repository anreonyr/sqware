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
//! 全盯上。搬出来之后**谁造镜像谁负责新鲜**：测试构建每一轮都造（`nu scripts/qtest.nu`），交互式由
//! `scripts/runner.nu` 先查在不在。那条"逐文件盯"的纪律连同它挡过的两类假象一起消失。
//!
//! 造出来的东西与内核**只共享一个约定**：initrd 落在内核 ELF **同目录**（`boot.nu` 就在那儿找）。
//! 内核也不再需要那两个数——它们写在区里前 8 字节（`env::manifest::PREAMBLE`），开机读。
//!
//! # 瘦身（`slim`）：为什么在**打包侧**剥符号与调试节
//!
//! `prog-*` 那份字节是**宿主调试用的那一份**（`gdb` 要符号），而进 initrd 的这一份只被内核
//! 按 ELF 段读——**两个消费者，两份字节**。在这里剥，两边都不亏。实测 31 张：debug
//! 128.2 MiB → 3.3 MiB（−97.4%），release 5.9 → 1.4 MiB（−77.2%）。省的不只是宿主读盘：
//! initrd 是 boot 给的**持久保留区**，帧分配器永不分配它（`platform/machine.rs` 的
//! `reserved`），debug 档那一份原先在 256 MiB 的机器上白占 78 MiB。
//!
//! `llvm-objcopy` 原样保留 `p_offset` / `p_vaddr` / `p_filesz`（仍页对齐），故**内核侧一行
//! 都不用改**——三条判据（`offset`/`vaddr` 页对齐、段落在文件内）在 93 张产物上逐张验过。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 目标三元组（与根 `.cargo/config.toml` 钉的那个一致）。
const TARGET: &str = "riscv64gc-unknown-none-elf";

/// 仓库根（本 crate 在 `crates/image`）。
fn root() -> PathBuf {
    let at = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    at.canonicalize().unwrap_or(at)
}

/// **唯一的宿主桥**：文本包含 `programs` 那份**宿主安全**的声明模块（`Program` 类型 +
/// `PROGRAMS` 注册表 + 各台自己的 `program.rs`）。
///
/// # 为什么是 `#[path]` 而不是一条依赖
///
/// 本 crate 是**宿主** std 程序，而 `programs` / `harness` 拖着 `protocol → runtime`
/// （riscv 内联汇编，宿主上不做代码生成就编不过）⇒ 它**不能**依赖 `programs`。
/// 而装配表只有一份——每台程序自己那份 `program.rs`。故那一份源码由**两侧各编一次**：
/// `programs` 编它给运行时用，本 crate 编它给打包用。
///
/// 它能成立的前提只有一条：`programs/src/program.rs` 与它 `#[path]` 拉进来的每一份声明
/// **只引 `env`**（宿主与 riscv 都编得过的那一层）。那条纪律写在 `programs/src/program.rs`
/// 的头注里——**这里就是它的报警器**：谁往声明里塞了 runtime / protocol 的引用，本 crate
/// 当场编不过。
#[allow(dead_code)]
#[path = "../../../programs/src/program.rs"]
mod program;

/// 认得的场景名——**从引导镜像那张表里收**（一个景存在 ⇔ 它有一条引导镜像），故不会与它脱节。
///
/// **照实记（本文件为什么只走 `Program` 上那四条窄面）**：装配声明拆成三块（身份 / 装配关系 /
/// 需求，见 `programs/src/program.rs` 的头注）之后，宿主这一侧的读者**一个字段都不许碰**——
/// 它只读"它是谁"那四样：`name()` / `space()` / `scenes()` / `entry()`。块再怎么挪，这四行不动。
/// （那四样里第三样从前叫 `kind()`——**它答的是空间（S/U）**，与"单元类型"同名不同事，改名见
/// `programs/src/program.rs` 的照实记。）
fn scenes() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for p in program::PROGRAMS {
        for s in p.entry() {
            if !out.contains(s) {
                out.push(s);
            }
        }
    }
    out
}

/// 这一景的引导镜像（哪一台的 `entry` 含这个景）。
fn entry_of(scene: &str) -> Option<&'static str> {
    program::PROGRAMS
        .iter()
        .find(|p| p.entry().contains(&scene))
        .map(|p| p.name())
}

/// 这一景要装的程序（**装配表按 `scenes` 过滤**；次序即装载次序）。
fn bins_for(scenario: &str) -> Result<Vec<(&'static str, env::ProgramKind)>, String> {
    let picked: Vec<(&'static str, env::ProgramKind)> = program::PROGRAMS
        .iter()
        // **两格滤**：进这一景（`scenes`）**且**是要起的服务（[`program::Kind::Service`]）。
        // 两格判的不是同一句话：`scenes` 说"这一景要不要它"，`kind` 说"**它有没有身子**"——
        // 名单里那个[目标单元](program::Kind::Target)（这一趟装配自己）没有身子，**不许去找
        // `prog-scene` 那样的 bin**；即使它将来写上了自己的景，这一格也照旧不装它。
        .filter(|p| p.scenes().contains(&scenario) && p.kind() == program::Kind::Service)
        .map(|p| (p.name(), p.space()))
        .collect();
    if picked.is_empty() {
        return Err(format!(
            "不认得的景 {scenario}（认得的：{}）",
            scenes().join(" / ")
        ));
    }
    Ok(picked)
}

/// **校验这一景的图，并返推导出的装配次序**（照实记：次序由各台声明里的 `after` 算出来，
/// **打包这一趟是它的第一个读者**——宿主上有名字、有退出码，坏图在这里就断掉）。
///
/// 三条话说得清：边指着本景没有的名字 / 被指着的那台没有"我答得动"的凭据 / 有环。
fn order_of(scenario: &str) -> Result<Vec<&'static str>, String> {
    let mut list: Vec<&'static program::Program> = program::PROGRAMS
        .iter()
        .copied()
        .filter(|p| p.scenes().contains(&scenario) && p.listed())
        .collect();
    program::order_scene(&mut list).map_err(|why| match why {
        program::DepsFail::Unknown(name) => {
            format!("景 {scenario} 的 deps 里有一个名字不在本景：{name}")
        }
        program::DepsFail::NoEvidence(name) => format!(
            "景 {scenario} 的 deps 指着 {name}，而它没有'我答得动'的凭据（`setup` 空）"
        ),
        program::DepsFail::Cycle(name) => {
            format!("景 {scenario} 的 deps 有环（取不出可排的台，卡在 {name}）")
        }
    })?;
    Ok(list.iter().map(|p| p.name()).collect())
}

/// 造这一景这一档的镜像，返 `initrd.img` 的落点（内核 ELF 同目录）。
pub fn build(scenario: &str, profile: &str) -> Result<PathBuf, String> {
    let root = root();
    let bins = bins_for(scenario)?;
    // **图那一趟**：先把这一景的 after 校验一遍（坏图当场断），顺手把算出来的次序打出来——
    // 验收要核对的就是这一行。
    println!(
        "initrd: 景 {scenario} 的装配次序：{}",
        order_of(scenario)?.join(" → ")
    );

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
    let status = Command::new(&cargo)
        .args(&args)
        .current_dir(&root)
        .status()
        .map_err(|e| format!("起不动嵌套 cargo：{e}"))?;
    if !status.success() {
        return Err("programs / harness 编不过（上面是它们自己的报错）".to_string());
    }

    // 读产物：**bin 名是约定** `prog-<名字>`（装配表里不写第二遍）；读进来就顺手瘦一遍
    // （`prog-*` 那份字节不动——它是宿主调试用的那一份，见文件头的"瘦身"那段）。
    let bin_dir = work.join(TARGET).join(profile);
    let mut images = Vec::with_capacity(bins.len());
    for (name, kind) in &bins {
        let bin = format!("prog-{name}");
        let elf = slim(&bin_dir.join(&bin), &work)?;
        images.push((*kind, *name, elf));
    }
    let items: Vec<(env::ProgramKind, &str, &[u8])> = images
        .iter()
        .map(|(kind, name, elf)| (*kind, *name, elf.as_slice()))
        .collect();
    // 引导镜像**按声明查**（不是"跟景同名"）：`product` 那一景的引导镜像仍是 `root`
    // （同一个引导域起两景，见 `root/program.rs` 的 `entry`）。它必须在清单里——不在就是
    // 那张装配表写错了。
    let entry = entry_of(scenario)
        .ok_or_else(|| format!("initrd: 不认得的景 {scenario}（认得的：{}）", scenes().join(" / ")))?;
    let root_at = bins
        .iter()
        .position(|(name, _)| *name == entry)
        .ok_or_else(|| format!("initrd: 景 {scenario} 的引导镜像 {entry} 不在这一景的清单里"))?;
    let blob = env::manifest::pack(&items, root_at)
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
    Ok(at)
}

/// 瘦一份程序镜像：剥掉符号与调试节，只留可装载内容（见文件头"瘦身"那段）。
///
/// `src` = `prog-<名字>`——**不动它**（宿主 `gdb` 要那份）；`out` = 那一档的工作目录
/// （临时文件落在它下面，随 `target/` 一起清理）。产瘦好的字节。
///
/// 临时名带 **pid**：并行的多个构建可能同档同程序（`cargo image` 与一次测试构建同时跑）
/// ——不带 pid 就会互相覆写。（**照实记**：这一句原先举的是 `cargo gate` 的 `examine`
/// 与 `product` 两门——那台已删。）
///
/// # Errors
///
/// 工具不在 PATH / 工具报错 / 瘦好的字节读不回来。**三条一律中止，绝不退回原字节**：
/// 静默退回的后果不是崩，而是那 97% 的收益悄悄消失，症状只剩"initrd 怎么还是这么大"。
fn slim(src: &Path, out: &Path) -> Result<Vec<u8>, String> {
    let name = src.file_name().and_then(|s| s.to_str()).unwrap_or("image");
    let dst = out.join(format!(".slim-{}-{name}", std::process::id()));
    // 失败路径也要收拾：临时文件不留痕（成功路径同样走这里）。
    let r = (|| {
        let status = Command::new("llvm-objcopy")
            .arg("--strip-all")
            .arg(src)
            .arg(&dst)
            .status()
            .map_err(|e| {
                format!(
                    "起不动 llvm-objcopy（{e}）——造镜像要它剥符号与调试节：装 llvm 包，或 \
                     `rustup component add llvm-tools` 之后把它放进 PATH"
                )
            })?;
        if !status.success() {
            return Err(format!("llvm-objcopy 对 {} 报错（{status}）", src.display()));
        }
        std::fs::read(&dst).map_err(|e| format!("读瘦好的 {} 失败：{e}", dst.display()))
    })();
    let _ = std::fs::remove_file(&dst);
    r
}
