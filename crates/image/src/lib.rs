//! image — **造镜像那一步**：编程序（`programs`，含测具那一档）→ 打 initrd → 放到内核 ELF 旁。
//!
//! 保留供用户态装载的 ELF 清单，并从引导 ELF 生成页化 capsule。
//! RX payload 包含完整补零页；内核只消费 capsule，不解析 ELF。
//! 入包 ELF 剥除符号与调试节，宿主调试产物仍保留这些内容。

mod capsule;

use std::path::{Path, PathBuf};
use std::process::Command;

/// 目标三元组（与根 `.cargo/config.toml` 钉的那个一致）。
const TARGET: &str = "riscv64gc-unknown-none-elf";

/// 仓库根（本 crate 在 `crates/image`）。
fn root() -> PathBuf {
    let at = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    at.canonicalize().unwrap_or(at)
}

/// **宿主声明表**：文本包含 `programs` 的宿主安全声明、Deployment 场景表和 Package 镜像表。
///
/// # 为什么是 `#[path]` 而不是一条依赖
///
/// 本 crate 是**宿主** std 程序，而 `programs` 依赖 execution
/// （riscv 内联汇编，宿主上不做代码生成就编不过）⇒ 它**不能**依赖 `programs`。
/// 各模块的 UnitFile 声明与镜像/部署选择分开；本 crate 只读取纯配置，不依赖运行时代码。
///
/// 装配声明只依赖 `env` 和纯 provider API crates（router / pipe / account）；这些 API 不依赖
/// execution 或 protocol。`programs/src/unit/mod.rs` 与这份宿主 crate 的依赖图一起检查这条边界。
#[allow(dead_code, unused_imports, reason = "Host projection of shared configuration and runtime reexports.")]
#[path = "../../../programs/src/unit/mod.rs"]
mod unit;

// Pure command resource declarations are shared with the Shell UnitFile. This host module only
// reads those declarations; it does not compile or run the Shell command loop.
#[allow(dead_code, reason = "Image reads bindings; Shell consumes command names and ports.")]
#[path = "../../../programs/src/user/shell/commands.rs"]
pub(crate) mod shell_commands;

mod user {
    pub(crate) mod shell {
        pub(crate) use crate::shell_commands as commands;
    }
}

/// 可打包的部署名。
fn scenes() -> Vec<&'static str> {
    unit::DEPLOYMENTS
        .iter()
        .map(|deployment| deployment.name)
        .collect()
}

/// 部署声明选定的引导镜像。
fn entry_of(scene: &str) -> Option<&'static str> {
    unit::Deployment::find(scene).and_then(unit::Deployment::entry_image)
}

/// Expand deployed UnitFiles, constructible UnitFiles, and their image resources into packages.
fn bins_with(
    deployment: &'static unit::Deployment,
    programs: &[&'static unit::UnitFile],
    packages: &[unit::Package],
) -> Result<Vec<(&'static str, env::ProgramKind)>, String> {
    let scenario = deployment.name;
    let mut names = Vec::new();
    fn add(names: &mut Vec<&'static str>, name: &'static str) {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let entry = programs
        .iter()
        .copied()
        .find(|program| program.name() == deployment.entry)
        .ok_or_else(|| format!("部署 {scenario} 入口 {} 没有 UnitFile", deployment.entry))?;
    let mut pending = Vec::new();
    let mut visited = Vec::new();
    fn enqueue(
        pending: &mut Vec<&'static unit::UnitFile>,
        visited: &mut Vec<&'static str>,
        program: &'static unit::UnitFile,
    ) {
        if !visited.contains(&program.name()) {
            visited.push(program.name());
            pending.push(program);
        }
    }
    enqueue(&mut pending, &mut visited, entry);
    for name in deployment.start {
        let program = programs
            .iter()
            .copied()
            .find(|program| program.name() == *name)
            .ok_or_else(|| format!("部署 {scenario} 的模块引用 {name} 没有 UnitFile"))?;
        enqueue(&mut pending, &mut visited, program);
    }
    while let Some(program) = pending.pop() {
        add(&mut names, program.loader.image);
        add_supply_images(&mut names, program, packages);
        for construction in program.control.construction {
            let unit::ConstructionSelector::Unit(name) = construction.selector else {
                continue;
            };
            let dependency = programs
                .iter()
                .copied()
                .find(|candidate| candidate.name() == name)
                .ok_or_else(|| {
                    format!(
                        "Unit {} 的 construction 引用了未知 UnitFile {name}",
                        program.name()
                    )
                })?;
            enqueue(&mut pending, &mut visited, dependency);
        }
    }
    for name in deployment.resources.iter().chain(deployment.sources) {
        add(&mut names, name);
    }

    // Image resources may themselves be referenced by modules sharing that image.
    let mut scanned = 0;
    while scanned < names.len() {
        let name = names[scanned];
        for program in programs
            .iter()
            .copied()
            .filter(|program| program.loader.image == name)
        {
            add_supply_images(&mut names, program, packages);
        }
        scanned += 1;
    }
    let entry_space = entry.loader.space;
    let entry_image = entry.loader.image;
    let picked: Vec<(&'static str, env::ProgramKind)> = names
        .into_iter()
        .map(|name| -> Result<_, String> {
            Ok(packages
                .iter()
                .find(|package| package.name == name)
                .map(|package| {
                    (
                        package.name,
                        if package.name == entry_image {
                            entry_space
                        } else {
                            env::ProgramKind::User
                        },
                    )
                })
                .ok_or_else(|| format!("景 {scenario} 引用了未登记镜像 {name}"))?)
        })
        .collect::<Result<_, _>>()?;
    if picked.is_empty() {
        return Err(format!(
            "不认得的景 {scenario}（认得的：{}）",
            scenes().join(" / ")
        ));
    }
    Ok(picked)
}

fn add_supply_images(
    names: &mut Vec<&'static str>,
    program: &unit::UnitFile,
    packages: &[unit::Package],
) {
    let initrd = system_api::supply::Key::of("system/initrd");
    for binding in program.supply() {
        if binding.source.mark() == initrd.mark()
            && binding.source.name() == initrd.name()
            && packages
                .iter()
                .any(|package| package.name == binding.resource.name())
        {
            add_name(names, binding.resource.name());
        }
    }
}

fn add_name(names: &mut Vec<&'static str>, name: &'static str) {
    if !names.contains(&name) {
        names.push(name);
    }
}

fn bins_for(
    deployment: &'static unit::Deployment,
) -> Result<Vec<(&'static str, env::ProgramKind)>, String> {
    bins_with(deployment, unit::PROGRAMS, unit::PACKAGES)
}

/// **校验这一景的图，并返推导出的装配次序**（次序由各台声明里的 `after` 算出来，
/// **打包这一趟是它的第一个读者**——宿主上有名字、有退出码，坏图在这里就断掉）。
///
/// 三条话说得清：边指着本景没有的名字 / 被指着的那台没有"我答得动"的凭据 / 有环。
fn order_of(
    deployment: &'static unit::Deployment,
    programs: &[&'static unit::UnitFile],
) -> Result<Vec<&'static str>, String> {
    let scenario = deployment.name;
    let mut list: Vec<&'static unit::UnitFile> = deployment
        .start
        .iter()
        .map(|name| {
            programs
                .iter()
                .copied()
                .find(|program| program.name() == *name)
                .ok_or_else(|| format!("部署 {scenario} 启动项 {name} 没有 UnitFile"))
        })
        .collect::<Result<_, _>>()?;
    unit::order_scene(&mut list, deployment.targets, deployment.bootstrap).map_err(
        |why| match why {
            unit::DepsFail::Unknown(name) => {
                format!("景 {scenario} 的 deps 里有一个名字不在本景：{name}")
            }
            unit::DepsFail::NoEvidence(name) => {
                format!("景 {scenario} 的 deps 指着 {name}，而它没有有效的 ready 声明")
            }
            unit::DepsFail::Cycle(name) => {
                format!("景 {scenario} 的 deps 有环（取不出可排的台，卡在 {name}）")
            }
            unit::DepsFail::Bootstrap(task) => {
                format!("景 {scenario} 缺少依赖的自举任务 {task:?}")
            }
            unit::DepsFail::DuplicateStart(name) => format!("景 {scenario} 重复启动 {name}"),
            unit::DepsFail::DuplicateTarget(name) => {
                format!("景 {scenario} 重复声明 Target {name}")
            }
        },
    )?;
    Ok(list.iter().map(|p| p.name()).collect())
}

/// 造这一景这一档的镜像，返 `initrd.img` 的落点（内核 ELF 同目录）。
pub fn build(scenario: &str, profile: &str) -> Result<PathBuf, String> {
    let root = root();
    let deployment = unit::Deployment::find(scenario)
        .ok_or_else(|| format!("不认得的景 {scenario}（认得的：{}）", scenes().join(" / ")))?;
    let bins = bins_for(deployment)?;
    // **图那一趟**：先把这一景的 after 校验一遍（坏图当场断），顺手把算出来的次序打出来——
    // 验收要核对的就是这一行。
    println!(
        "initrd: 景 {scenario} 的装配次序：{}",
        order_of(deployment, unit::PROGRAMS)?.join(" → ")
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

    // Read and strip the binary registered by this Package.
    // （`prog-*` 那份字节不动——它是宿主调试用的那一份，见文件头的"瘦身"那段）。
    let bin_dir = work.join(TARGET).join(profile);
    let mut images = Vec::with_capacity(bins.len());
    for (name, kind) in &bins {
        let package = unit::PACKAGES
            .iter()
            .find(|package| package.name == *name)
            .ok_or_else(|| format!("部署镜像登记丢失：{name}"))?;
        let elf = slim(&bin_dir.join(package.binary), &work)?;
        images.push((*kind, *name, elf));
    }
    let items: Vec<(env::ProgramKind, &str, &[u8])> = images
        .iter()
        .map(|(kind, name, elf)| (*kind, *name, elf.as_slice()))
        .collect();
    // 引导镜像由 entry 声明选择，并且必须包含在本景清单中。
    let entry = entry_of(scenario)
        .ok_or_else(|| format!("initrd: deployment entry image missing {scenario}"))?;
    let entry_at = bins
        .iter()
        .position(|(name, _)| *name == entry)
        .ok_or_else(|| format!("initrd: 景 {scenario} 的引导镜像 {entry} 不在这一景的清单里"))?;
    let mut blob = env::ledger::manifest::pack(&items, entry_at, deployment.name)
        .ok_or_else(|| "initrd: 非法部署或镜像清单".to_string())?;
    let capsule = capsule::encode(items[entry_at].2)
        .map_err(|error| format!("bootstrap {entry}: {error:?}"))?;
    let offset = blob
        .len()
        .checked_next_multiple_of(env::ledger::capsule::PAGE)
        .ok_or_else(|| "initrd: capsule offset overflow".to_string())?;
    let total = offset
        .checked_add(capsule.len())
        .filter(|n| *n <= u32::MAX as usize)
        .ok_or_else(|| "initrd: capsule exceeds u32 range".to_string())?;
    blob.try_reserve(total - blob.len())
        .map_err(|_| "initrd: no memory".to_string())?;
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
/// `src` is the Package binary; it stays unchanged for host debugging. `out` is the profile work directory.
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
            return Err(format!(
                "llvm-objcopy 对 {} 报错（{status}）",
                src.display()
            ));
        }
        std::fs::read(&dst).map_err(|e| format!("读瘦好的 {} 失败：{e}", dst.display()))
    })();
    let _ = std::fs::remove_file(&dst);
    r
}

#[cfg(test)]
mod tests {
    use super::{bins_for, bins_with, order_of, unit};

    #[test]
    fn deployment_image_and_start_members_are_registered() {
        for deployment in unit::DEPLOYMENTS {
            let images = bins_for(deployment).unwrap();
            order_of(deployment, unit::PROGRAMS).unwrap();
            let entry_image = deployment.entry_image().unwrap();
            assert!(images.iter().any(|(name, _)| *name == entry_image));
            for (image, _) in &images {
                assert!(unit::PACKAGES.iter().any(|package| package.name == *image));
            }
            for start in deployment.start {
                let program = unit::PROGRAMS
                    .iter()
                    .find(|program| program.name() == *start)
                    .unwrap();
                assert!(
                    images
                        .iter()
                        .any(|(image, _)| image == &program.loader.image)
                );
            }
        }
        let fault = unit::Deployment::find("system-fault").unwrap();
        let fault_bins = bins_for(fault).unwrap();
        for required in ["shell", "cat", "emit", "upper", "fail", "spin", "workers"] {
            assert!(fault_bins.iter().any(|(image, _)| *image == required));
        }
        assert!(!fault.start.contains(&"shell"));
        assert!(!order_of(fault, unit::PROGRAMS).unwrap().contains(&"shell"));

        let product = bins_for(unit::Deployment::find("product").unwrap())
            .unwrap()
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>();
        for supplied in ["shell", "cat", "emit", "upper", "fail", "spin", "workers"] {
            assert!(product.contains(&supplied));
        }
    }

    #[test]
    fn package_names_are_unique_and_exclude_scene_targets() {
        for (index, package) in unit::PACKAGES.iter().enumerate() {
            assert!(!package.name.is_empty());
            assert!(package.binary.starts_with("prog-"));
            assert!(unit::PACKAGES[..index]
                .iter()
                .all(|earlier| earlier.name != package.name && earlier.binary != package.binary));
        }
        assert!(!unit::PACKAGES.iter().any(|package| package.name == "scene"));
    }

    #[test]
    fn invalid_image_and_start_references_fail_on_the_build_paths() {
        static MISSING_IMAGE: unit::Deployment = unit::Deployment {
            name: "invalid-image",
            entry: "accept-system",
            start: &[],
            guarded: &[],
            resources: &["missing"],
            targets: &[],
            bootstrap: &[],
            sources: &[],
        };
        assert!(
            bins_for(&MISSING_IMAGE)
                .unwrap_err()
                .contains("未登记镜像 missing")
        );

        static MISSING_START: unit::Deployment = unit::Deployment {
            name: "invalid-start",
            entry: "accept-system",
            start: &["missing"],
            guarded: &[],
            resources: &[],
            targets: &[],
            bootstrap: &[],
            sources: &[],
        };
        assert!(
            order_of(&MISSING_START, unit::PROGRAMS)
                .unwrap_err()
                .contains("启动项 missing 没有 UnitFile")
        );
    }

    #[test]
    fn dependency_cycle_fails_through_the_build_order_path() {
        static A: unit::UnitFile = unit::UnitFile {
            name: "cycle-a",
            loader: unit::LoaderConfig::user("cycle-a"),
            operator: unit::OperatorConfig::DEFAULT,
            identity: unit::Identity {
                aliases: &[],
                grants: &[],
            },
            control: unit::ControlConfig {
                after: Some(&[unit::Dependency::Unit("cycle-b")]),
                ending: Some(unit::Ending::Transient),
                ready: unit::Ready::OnSpawn,
                construction: &[],
                retirement: &[],
            },
            supply: unit::SupplyConfig { bindings: &[] },
        };
        static B: unit::UnitFile = unit::UnitFile {
            name: "cycle-b",
            loader: unit::LoaderConfig::user("cycle-b"),
            operator: unit::OperatorConfig::DEFAULT,
            identity: unit::Identity {
                aliases: &[],
                grants: &[],
            },
            control: unit::ControlConfig {
                after: Some(&[unit::Dependency::Target(unit::SCENE)]),
                ending: Some(unit::Ending::Transient),
                ready: unit::Ready::OnSpawn,
                construction: &[],
                retirement: &[],
            },
            supply: unit::SupplyConfig { bindings: &[] },
        };
        static TARGETS: &[unit::Target] = &[unit::Target {
            name: unit::SCENE,
            members: &["cycle-a"],
        }];
        static CYCLE: unit::Deployment = unit::Deployment {
            name: "cycle",
            entry: "cycle-a",
            start: &["cycle-a", "cycle-b"],
            guarded: &[],
            resources: &[],
            targets: TARGETS,
            bootstrap: &[],
            sources: &[],
        };
        assert!(order_of(&CYCLE, &[&A, &B]).unwrap_err().contains("有环"));
    }

    #[test]
    fn package_closure_allows_unitless_images_and_shared_loader_images() {
        static ENTRY_BINDINGS: &[unit::Binding] = &[
            unit::Binding {
                source: system_api::supply::Key::of("system/initrd"),
                resource: system_api::supply::Key::of("supplied-image"),
                slot: env::Mark::of("test/supplied-image"),
                permission: env::Permission::FETCH,
            },
            unit::Binding {
                source: system_api::supply::Key::of("system/initrd"),
                resource: system_api::supply::Key::of("runtime-data"),
                slot: env::Mark::of("test/runtime-data"),
                permission: env::Permission::FETCH,
            },
            unit::Binding {
                source: system_api::supply::Key::of("system/operator"),
                resource: system_api::supply::Key::of("not-an-image"),
                slot: env::Mark::of("test/not-an-image"),
                permission: env::Permission::FETCH,
            },
        ];
        static ENTRY: unit::UnitFile = unit::UnitFile {
            name: "test-entry",
            loader: unit::LoaderConfig::supervisor("shared-image"),
            operator: unit::OperatorConfig::DEFAULT,
            identity: unit::Identity {
                aliases: &[],
                grants: &[],
            },
            control: unit::ControlConfig {
                after: None,
                ending: None,
                ready: unit::Ready::OnSpawn,
                construction: &[unit::Construction::image()],
                retirement: &[],
            },
            supply: unit::SupplyConfig {
                bindings: ENTRY_BINDINGS,
            },
        };
        static FIRST: unit::UnitFile = unit::UnitFile {
            name: "test-first",
            loader: unit::LoaderConfig::user("shared-image"),
            operator: unit::OperatorConfig::DEFAULT,
            identity: unit::Identity {
                aliases: &[],
                grants: &[],
            },
            control: unit::ControlConfig {
                after: None,
                ending: None,
                ready: unit::Ready::OnSpawn,
                construction: &[unit::Construction::unit("test-second")],
                retirement: &[],
            },
            supply: unit::SupplyConfig { bindings: &[] },
        };
        static SECOND: unit::UnitFile = unit::UnitFile {
            name: "test-second",
            loader: unit::LoaderConfig::user("shared-image"),
            operator: unit::OperatorConfig::DEFAULT,
            identity: unit::Identity {
                aliases: &[],
                grants: &[],
            },
            control: unit::ControlConfig {
                after: None,
                ending: None,
                ready: unit::Ready::OnSpawn,
                construction: &[unit::Construction::unit("test-first")],
                retirement: &[],
            },
            supply: unit::SupplyConfig { bindings: &[] },
        };
        static DEPLOYMENT: unit::Deployment = unit::Deployment {
            name: "shared",
            entry: "test-entry",
            start: &["test-first", "test-second"],
            guarded: &[],
            resources: &["unitless-image"],
            targets: &[],
            bootstrap: &[],
            sources: &[],
        };
        static PACKAGES: &[unit::Package] = &[
            unit::Package {
                name: "shared-image",
                binary: "shared-elf",
            },
            unit::Package {
                name: "unitless-image",
                binary: "raw-elf",
            },
            unit::Package {
                name: "supplied-image",
                binary: "supplied-elf",
            },
        ];

        let bins = bins_with(&DEPLOYMENT, &[&ENTRY, &FIRST, &SECOND], PACKAGES).unwrap();
        assert_eq!(
            bins,
            [
                ("shared-image", env::ProgramKind::Supervisor),
                ("supplied-image", env::ProgramKind::User),
                ("unitless-image", env::ProgramKind::User),
            ]
        );
    }

    #[test]
    fn construction_unit_references_must_resolve() {
        static ENTRY: unit::UnitFile = unit::UnitFile {
            name: "unknown-entry",
            loader: unit::LoaderConfig::user("unknown-entry"),
            operator: unit::OperatorConfig::DEFAULT,
            identity: unit::Identity {
                aliases: &[],
                grants: &[],
            },
            control: unit::ControlConfig {
                after: None,
                ending: None,
                ready: unit::Ready::OnSpawn,
                construction: &[unit::Construction::unit("missing-unit")],
                retirement: &[],
            },
            supply: unit::SupplyConfig { bindings: &[] },
        };
        static DEPLOYMENT: unit::Deployment = unit::Deployment {
            name: "unknown-construction",
            entry: "unknown-entry",
            start: &[],
            guarded: &[],
            resources: &[],
            targets: &[],
            bootstrap: &[],
            sources: &[],
        };
        static PACKAGES: &[unit::Package] = &[unit::Package {
            name: "unknown-entry",
            binary: "unknown-entry-elf",
        }];

        let error = bins_with(&DEPLOYMENT, &[&ENTRY], PACKAGES).unwrap_err();
        assert!(error.contains("未知 UnitFile missing-unit"));
    }

    #[test]
    fn raw_image_construction_does_not_require_another_unit_file() {
        static ENTRY: unit::UnitFile = unit::UnitFile {
            name: "raw-entry",
            loader: unit::LoaderConfig::user("raw-entry"),
            operator: unit::OperatorConfig::DEFAULT,
            identity: unit::Identity {
                aliases: &[],
                grants: &[],
            },
            control: unit::ControlConfig {
                after: None,
                ending: None,
                ready: unit::Ready::OnSpawn,
                construction: &[unit::Construction::image()],
                retirement: &[],
            },
            supply: unit::SupplyConfig { bindings: &[] },
        };
        static DEPLOYMENT: unit::Deployment = unit::Deployment {
            name: "raw-construction",
            entry: "raw-entry",
            start: &[],
            guarded: &[],
            resources: &[],
            targets: &[],
            bootstrap: &[],
            sources: &[],
        };
        static PACKAGES: &[unit::Package] = &[unit::Package {
            name: "raw-entry",
            binary: "raw-entry-elf",
        }];

        assert_eq!(
            bins_with(&DEPLOYMENT, &[&ENTRY], PACKAGES).unwrap(),
            [("raw-entry", env::ProgramKind::User)]
        );
    }

    #[test]
    fn missing_bootstrap_dependency_fails_through_the_build_order_path() {
        static UNIT: unit::UnitFile = unit::UnitFile {
            name: "bootstrap-check",
            loader: unit::LoaderConfig::user("bootstrap-check"),
            operator: unit::OperatorConfig::DEFAULT,
            identity: unit::Identity {
                aliases: &[],
                grants: &[],
            },
            control: unit::ControlConfig {
                after: Some(&[unit::Dependency::Bootstrap(unit::Bootstrap::Identity)]),
                ending: Some(unit::Ending::Transient),
                ready: unit::Ready::OnSpawn,
                construction: &[],
                retirement: &[],
            },
            supply: unit::SupplyConfig { bindings: &[] },
        };
        static DEPLOYMENT: unit::Deployment = unit::Deployment {
            name: "bootstrap-check",
            entry: "bootstrap-check",
            start: &["bootstrap-check"],
            guarded: &[],
            resources: &[],
            targets: &[],
            bootstrap: &[],
            sources: &[],
        };
        assert!(
            order_of(&DEPLOYMENT, &[&UNIT])
                .unwrap_err()
                .contains("Identity")
        );
    }

    #[test]
    fn target_dependencies_participate_in_cycle_and_duplicate_validation() {
        static A: unit::UnitFile = unit::UnitFile {
            name: "target-a",
            loader: unit::LoaderConfig::user("target-a"),
            operator: unit::OperatorConfig::DEFAULT,
            identity: unit::Identity {
                aliases: &[],
                grants: &[],
            },
            control: unit::ControlConfig {
                after: Some(&[unit::Dependency::Target("after-b")]),
                ending: Some(unit::Ending::Transient),
                ready: unit::Ready::OnSpawn,
                construction: &[],
                retirement: &[],
            },
            supply: unit::SupplyConfig { bindings: &[] },
        };
        static B: unit::UnitFile = unit::UnitFile {
            name: "target-b",
            loader: unit::LoaderConfig::user("target-b"),
            operator: unit::OperatorConfig::DEFAULT,
            identity: unit::Identity {
                aliases: &[],
                grants: &[],
            },
            control: unit::ControlConfig {
                after: Some(&[unit::Dependency::Target("after-a")]),
                ending: Some(unit::Ending::Transient),
                ready: unit::Ready::OnSpawn,
                construction: &[],
                retirement: &[],
            },
            supply: unit::SupplyConfig { bindings: &[] },
        };
        static TARGETS: &[unit::Target] = &[
            unit::Target {
                name: "after-a",
                members: &["target-a"],
            },
            unit::Target {
                name: "after-b",
                members: &["target-b"],
            },
        ];
        static CYCLE: unit::Deployment = unit::Deployment {
            name: "target-cycle",
            entry: "target-a",
            start: &["target-a", "target-b"],
            guarded: &[],
            resources: &[],
            targets: TARGETS,
            bootstrap: &[],
            sources: &[],
        };
        assert!(order_of(&CYCLE, &[&A, &B]).unwrap_err().contains("有环"));

        static DUP_START: unit::Deployment = unit::Deployment {
            name: "duplicate-start",
            entry: "target-a",
            start: &["target-a", "target-a"],
            guarded: &[],
            resources: &[],
            targets: &[],
            bootstrap: &[],
            sources: &[],
        };
        assert!(
            order_of(&DUP_START, &[&A])
                .unwrap_err()
                .contains("重复启动")
        );

        static DUP_TARGETS: &[unit::Target] = &[
            unit::Target {
                name: "same",
                members: &["target-a"],
            },
            unit::Target {
                name: "same",
                members: &["target-b"],
            },
        ];
        static DUP_TARGET: unit::Deployment = unit::Deployment {
            name: "duplicate-target",
            entry: "target-a",
            start: &["target-a"],
            guarded: &[],
            resources: &[],
            targets: DUP_TARGETS,
            bootstrap: &[],
            sources: &[],
        };
        assert!(
            order_of(&DUP_TARGET, &[&A])
                .unwrap_err()
                .contains("重复声明 Target")
        );
    }

    #[test]
    fn selecting_a_deployment_rejects_unknown_names_and_mismatched_entries() {
        let entry = env::ledger::manifest::Entry {
            kind: env::ProgramKind::Supervisor,
            name: "system",
            elf: b"elf",
        };
        for name in ["", "unknown", "system", "system-iommu"] {
            assert!(matches!(
                unit::select_deployment(name, entry),
                Err(unit::DeploymentSelectionError::Unknown)
            ));
        }
        assert!(matches!(
            unit::select_deployment("accept", entry),
            Err(unit::DeploymentSelectionError::EntryMismatch)
        ));
        assert!(matches!(
            unit::select_deployment("product", env::ledger::manifest::Entry {
                kind: env::ProgramKind::User,
                ..entry
            }),
            Err(unit::DeploymentSelectionError::EntryMismatch)
        ));
    }

    #[test]
    fn guarded_deployments_name_only_started_catalog_programs() {
        for deployment in unit::DEPLOYMENTS {
            assert!(
                deployment.guarded().is_ok(),
                "deployment {} has an invalid guard selection",
                deployment.name
            );
        }
    }

    #[test]
    fn packaged_selection_roundtrips_for_every_deployment() {
        for deployment in unit::DEPLOYMENTS {
            let images = bins_for(deployment).unwrap();
            let items: Vec<_> = images.iter()
                .map(|(name, kind)| (*kind, *name, b"actual image placeholder".as_slice()))
                .collect();
            let entry_at = images.iter()
                .position(|(name, _)| Some(*name) == deployment.entry_image()).unwrap();
            let blob = env::ledger::manifest::pack(&items, entry_at, deployment.name).unwrap();
            let entries = env::ledger::manifest::Entries::new(&blob).unwrap();
            let selected = unit::select_deployment(entries.deployment(), entries.entry().unwrap()).unwrap();
            assert_eq!(unit::DEPLOYMENTS.iter().filter(|other| other.name == deployment.name).count(), 1);
            assert_eq!(selected.name, deployment.name);
            assert_eq!(selected.entry, deployment.entry);
            assert_eq!(selected.start, deployment.start);
            assert_eq!(entries.count(), items.len());
        }
    }

    #[test]
    fn long_soak_uses_loader_arguments_without_another_controller_image() {
        let deployment = unit::Deployment::find("iommu-soak-long").unwrap();
        assert_eq!(deployment.entry, "system");
        assert!(deployment.start.contains(&"device-ctl-soak-long"));
        let controller = unit::PROGRAMS.iter()
            .find(|program| program.name() == "device-ctl-soak-long").unwrap();
        assert_eq!(controller.loader.image, "device-ctl-soak");
        assert_eq!(controller.loader.args, &[1024]);
        let bins = bins_for(deployment).unwrap();
        assert_eq!(bins.iter().filter(|(name, _)| *name == "device-ctl-soak").count(), 1);
        assert!(!bins.iter().any(|(name, _)| *name == "device-ctl-soak-long"));
    }

    #[test]
    fn product_deployments_share_one_system_image_without_losing_their_start_lists() {
        let variants = [
            ("product", None),
            ("iommu-product", Some("device-ctl")),
            ("iommu-product-fault", Some("device-ctl-fault")),
            ("iommu-product-before", Some("device-ctl-before")),
            ("iommu-product-after", Some("device-ctl-after")),
        ];
        for (name, controller) in variants {
            let deployment = unit::Deployment::find(name).unwrap();
            assert_eq!(deployment.entry, "system");
            assert_eq!(deployment.entry_image(), Some("system"));
            let bins = bins_for(deployment).unwrap();
            assert_eq!(bins.iter().filter(|(name, _)| *name == "system").count(), 1);
            assert!(!bins.iter().any(|(name, _)| name.starts_with("system-iommu")));
            for candidate in ["device-ctl", "device-ctl-fault", "device-ctl-before", "device-ctl-after"] {
                assert_eq!(deployment.start.contains(&candidate), controller == Some(candidate));
            }
        }
        assert!(!unit::PACKAGES.iter().any(|package| package.name.starts_with("system-iommu")));
        assert!(!unit::PROGRAMS.iter().any(|program| program.name().starts_with("system-iommu")));
    }

    #[test]
    fn deployment_selection_does_not_depend_on_other_entry_images_being_present() {
        use env::ledger::manifest::{Entries, pack};
        let items: &[(env::ProgramKind, &str, &[u8])] = &[
            (env::ProgramKind::Supervisor, "accept-system", b"accept elf"),
            (env::ProgramKind::Supervisor, "system", b"system elf"),
            (env::ProgramKind::User, "device-ctl-fault", b"controller elf"),
        ];
        let blob = pack(items, 1, "iommu-product-fault").unwrap();
        let entries = Entries::new(&blob).unwrap();
        assert_eq!(
            unit::select_deployment(entries.deployment(), entries.entry().unwrap()).unwrap().name,
            "iommu-product-fault",
        );
        let wrong = pack(items, 0, "iommu-product-fault").unwrap();
        let entries = Entries::new(&wrong).unwrap();
        assert!(matches!(
            unit::select_deployment(entries.deployment(), entries.entry().unwrap()),
            Err(unit::DeploymentSelectionError::EntryMismatch)
        ));
    }
}
