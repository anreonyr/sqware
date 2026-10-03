//! image — **造镜像那一步**：编程序（`programs`，含测具那一档）→ 打 initrd → 放到内核 ELF 旁。
//!
//! 保留供用户态装载的 ELF 清单，并从引导 ELF 生成页化 capsule。
//! RX payload 包含完整补零页；内核只消费 capsule，不解析 ELF。
//! 入包 ELF 剥除符号与调试节，宿主调试产物仍保留这些内容。

use std::path::{Path, PathBuf};
use std::process::Command;

/// 目标三元组（与根 `.cargo/config.toml` 钉的那个一致）。
const TARGET: &str = "riscv64gc-unknown-none-elf";

/// 仓库根（本 crate 在 `crates/image`）。
fn root() -> PathBuf {
    let at = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    at.canonicalize().unwrap_or(at)
}

/// **唯一的宿主桥**：文本包含 `programs` 那份**宿主安全**的声明模块（`UnitFile` 类型 +
/// `PROGRAMS` 注册表 + 各台自己的 `program.rs`）。
///
/// # 为什么是 `#[path]` 而不是一条依赖
///
/// 本 crate 是**宿主** std 程序，而 `programs` 拖着 `protocol → runtime`
/// （riscv 内联汇编，宿主上不做代码生成就编不过）⇒ 它**不能**依赖 `programs`。
/// 而装配表只有一份——每台程序自己那份 `program.rs`。故那一份源码由**两侧各编一次**：
/// `programs` 编它给运行时用，本 crate 编它给打包用。
///
/// 它能成立的前提只有一条：`programs/src/unit/` 与它 `#[path]` 拉进来的每一份声明
/// **只引 `env`**（宿主与 riscv 都编得过的那一层）。那条纪律写在 `programs/src/unit/mod.rs`
/// 的头注里——**这里就是它的报警器**：谁往声明里塞了 runtime / protocol 的引用，本 crate
/// 当场编不过。
#[allow(dead_code)]
#[path = "../../../programs/src/unit/mod.rs"]
mod unit;

/// 认得的场景名——**从引导镜像那张表里收**（一个景存在 ⇔ 它有一条引导镜像），故不会与它脱节。
fn scenes() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for p in unit::PROGRAMS {
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
    unit::PROGRAMS
        .iter()
        .find(|p| p.entry().contains(&scene))
        .map(|p| p.name())
}

/// 这一景要装的程序（**装配表按 `wanted_by` 过滤**；次序即装载次序）。
fn bins_for(scenario: &str) -> Result<Vec<(&'static str, env::ProgramKind)>, String> {
    let picked: Vec<(&'static str, env::ProgramKind)> = unit::PROGRAMS
        .iter()
        // **两格滤**：进这一景（`wanted_by`）**且**是要起的服务（[`unit::Kind::Service`]）。
        // 两格判的不是同一句话：`wanted_by` 说"这一景要不要它"，`kind` 说"**它有没有身子**"——
        // 名单里那个[目标单元](unit::Kind::Target)（这一趟装配自己）没有身子，**不许去找
        // `prog-scene` 那样的 bin**；即使它将来写上了自己的景，这一格也照旧不装它。
        .filter(|p| p.wanted_by().contains(&scenario) && p.kind() == unit::Kind::Service)
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

/// **校验这一景的图，并返推导出的装配次序**（次序由各台声明里的 `after` 算出来，
/// **打包这一趟是它的第一个读者**——宿主上有名字、有退出码，坏图在这里就断掉）。
///
/// 三条话说得清：边指着本景没有的名字 / 被指着的那台没有"我答得动"的凭据 / 有环。
fn order_of(scenario: &str) -> Result<Vec<&'static str>, String> {
    let mut list: Vec<&'static unit::UnitFile> = unit::PROGRAMS
        .iter()
        .copied()
        .filter(|p| p.wanted_by().contains(&scenario) && p.listed())
        .collect();
    unit::order_scene(&mut list).map_err(|why| match why {
        unit::DepsFail::Unknown(name) => {
            format!("景 {scenario} 的 deps 里有一个名字不在本景：{name}")
        }
        unit::DepsFail::NoEvidence(name) => format!(
            "景 {scenario} 的 deps 指着 {name}，而它没有'我答得动'的凭据（`setup` 空）"
        ),
        unit::DepsFail::Cycle(name) => {
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
    // 同目录再起 cargo 会互锁死等（这一格是从 `kernel/build.rs` 原样搬过来的）。
    let work = root.join("target/image").join(profile);
    let mut args = vec![
        "build".to_string(),
        // **一个包**：产品与测具同住 `programs`（`src/` 与 `src/harness/`）。
        "-p".to_string(),
        "programs".to_string(),
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
        return Err("programs 编不过（上面是它自己的报错）".to_string());
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
    // 引导镜像由 entry 声明选择，并且必须包含在本景清单中。
    let entry = entry_of(scenario)
        .ok_or_else(|| format!("initrd: 不认得的景 {scenario}（认得的：{}）", scenes().join(" / ")))?;
    let entry_at = bins
        .iter()
        .position(|(name, _)| *name == entry)
        .ok_or_else(|| format!("initrd: 景 {scenario} 的引导镜像 {entry} 不在这一景的清单里"))?;
    let mut blob = env::ledger::manifest::pack(&items, entry_at)
        .ok_or_else(|| "initrd: 清单越界（条数 / 名字长度 / 空镜像）".to_string())?;
    let capsule = loader::capsule(items[entry_at].2)
        .map_err(|error| format!("bootstrap {entry}: {error:?}"))?;
    let offset = blob.len().checked_next_multiple_of(env::ledger::capsule::PAGE)
        .ok_or_else(|| "initrd: capsule offset overflow".to_string())?;
    let total = offset.checked_add(capsule.len()).filter(|n| *n <= u32::MAX as usize)
        .ok_or_else(|| "initrd: capsule exceeds u32 range".to_string())?;
    blob.try_reserve(total - blob.len()).map_err(|_| "initrd: no memory".to_string())?;
    blob.resize(offset, 0);
    blob.extend_from_slice(&capsule);
    blob[..4].copy_from_slice(&(offset as u32).to_le_bytes());
    blob[4..8].copy_from_slice(&(capsule.len() as u32).to_le_bytes());

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
