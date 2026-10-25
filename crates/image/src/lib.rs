//! image — **造镜像那一步**：编程序（`programs`，含测具那一档）→ 打 initrd → 放到内核 ELF 旁。
//!
//! 保留供用户态装载的 ELF 清单，并从引导 ELF 生成页化 capsule。
//! RX payload 包含完整补零页；内核只消费 capsule，不解析 ELF。
//! 入包 ELF 剥除符号与调试节，宿主调试产物仍保留这些内容。

mod capsule;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use unit::{Deployment, UnitFile};

pub use unit::ImageBuild;

/// Expand declared UnitFiles and their image resources into the requested
/// image inventory. Only `ImageBuild.image` is used to identify an image.
fn bins_with(
    deployment: &'static Deployment,
    programs: &[&'static UnitFile],
    builds: &[ImageBuild],
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
        pending: &mut Vec<&'static UnitFile>,
        visited: &mut Vec<&'static str>,
        program: &'static UnitFile,
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
        add_supply_images(&mut names, program, builds);
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
            add_supply_images(&mut names, program, builds);
        }
        scanned += 1;
    }
    let entry_space = entry.loader.space;
    let entry_image = entry.loader.image;
    let picked: Vec<(&'static str, env::ProgramKind)> = names
        .into_iter()
        .map(|name| -> Result<_, String> {
            let build = builds
                .iter()
                .find(|build| build.image == name)
                .ok_or_else(|| format!("景 {scenario} 引用了未登记镜像 {name}"))?;
            Ok((
                build.image,
                if build.image == entry_image {
                    entry_space
                } else {
                    env::ProgramKind::User
                },
            ))
        })
        .collect::<Result<_, _>>()?;
    if picked.is_empty() {
        return Err(format!("景 {scenario} 没有可构建镜像"));
    }
    Ok(picked)
}

fn add_supply_images(
    names: &mut Vec<&'static str>,
    program: &UnitFile,
    builds: &[ImageBuild],
) {
    let initrd = system_api::supply::Key::of("system/initrd");
    for binding in program.supply() {
        if binding.source.mark() == initrd.mark()
            && binding.source.name() == initrd.name()
            && builds
                .iter()
                .any(|build| build.image == binding.resource.name())
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

/// **校验这一景的图，并返推导出的装配次序**（次序由各台声明里的 `after` 算出来，
/// **打包这一趟是它的第一个读者**——宿主上有名字、有退出码，坏图在这里就断掉）。
///
/// 三条话说得清：边指着本景没有的名字 / 被指着的那台没有"我答得动"的凭据 / 有环。
fn order_of(
    deployment: &'static Deployment,
    programs: &[&'static UnitFile],
) -> Result<Vec<&'static str>, String> {
    let scenario = deployment.name;
    let mut list: Vec<&'static UnitFile> = deployment
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

/// Resolve the real Cargo executable artifacts for explicit image targets.
///
/// `root` is the caller's build root (manifest paths are relative to it) and
/// `work` is the nested Cargo target directory. Cargo, not this library,
/// resolves targets, profiles and features.
pub fn executables(
    root: &Path,
    work: &Path,
    builds: &[ImageBuild],
) -> Result<Vec<(&'static str, PathBuf)>, String> {
    validate_builds(builds)?;
    let picked: Vec<&ImageBuild> = builds.iter().collect();
    let mut artifacts = Vec::new();
    for (index, group) in groups(&picked).into_iter().enumerate() {
        let group_work = work.join(format!("group-{index}"));
        artifacts.extend(build_group(root, &group_work, &group)?);
    }
    Ok(artifacts)
}

/// Build one deployment from explicit declarations, deployment and Cargo image targets.
///
/// The caller supplies the build root, nested target directory and final output
/// path. No repository or composition path is inferred here.
pub fn build_with(
    root: &Path,
    work: &Path,
    output: &Path,
    declarations: &[&'static UnitFile],
    deployment: &'static Deployment,
    builds: &[ImageBuild],
) -> Result<PathBuf, String> {
    validate_builds(builds)?;
    let bins = bins_with(deployment, declarations, builds)?;
    println!(
        "initrd: 景 {} 的装配次序：{}",
        deployment.name,
        order_of(deployment, declarations)?.join(" → ")
    );
    let picked = needed_builds(&bins, builds)?;

    let mut artifacts = Vec::new();
    for (index, group) in groups(&picked).into_iter().enumerate() {
        let group_work = work.join(format!("group-{index}"));
        artifacts.extend(build_group(root, &group_work, &group)?);
    }

    let mut images = Vec::with_capacity(bins.len());
    for (name, kind) in &bins {
        let executable = artifacts
            .iter()
            .find(|(image, _)| image == name)
            .map(|(_, path)| path)
            .ok_or_else(|| format!("Cargo 未产出镜像 {name} 的可执行文件"))?;
        let elf = slim(executable, work)?;
        images.push((*kind, *name, elf));
    }
    let items: Vec<(env::ProgramKind, &str, &[u8])> = images
        .iter()
        .map(|(kind, name, elf)| (*kind, *name, elf.as_slice()))
        .collect();
    let entry = deployment
        .entry_image(declarations)
        .ok_or_else(|| format!("initrd: deployment entry image missing {}", deployment.name))?;
    let entry_at = bins
        .iter()
        .position(|(name, _)| *name == entry)
        .ok_or_else(|| {
            format!(
                "initrd: 景 {} 的引导镜像 {entry} 不在这一景的清单里",
                deployment.name
            )
        })?;
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

    if let Some(dir) = output.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("建 {} 失败：{e}", dir.display()))?;
    }
    std::fs::write(output, &blob).map_err(|e| format!("写 {} 失败：{e}", output.display()))?;
    let at = std::fs::canonicalize(output).unwrap_or_else(|_| output.to_path_buf());
    println!(
        "initrd packed: {} ({} B, {} programs)",
        at.display(),
        blob.len(),
        bins.len()
    );
    Ok(at)
}

fn validate_builds(builds: &[ImageBuild]) -> Result<(), String> {
    for (index, build) in builds.iter().enumerate() {
        if build.image.is_empty() || build.package.is_empty() || build.bin.is_empty() {
            return Err("构建映射缺少 image/package/bin".to_string());
        }
        if builds[..index]
            .iter()
            .any(|earlier| earlier.image == build.image)
        {
            return Err(format!("构建映射重复镜像 {}", build.image));
        }
    }
    Ok(())
}

fn needed_builds<'a>(
    bins: &[(&'static str, env::ProgramKind)],
    builds: &'a [ImageBuild],
) -> Result<Vec<&'a ImageBuild>, String> {
    let mut picked = Vec::with_capacity(bins.len());
    for (image, _) in bins {
        let mut found = None;
        for build in builds {
            if build.image != *image {
                continue;
            }
            if found.is_some() {
                return Err(format!("镜像 {image} 在构建映射中不唯一"));
            }
            found = Some(build);
        }
        picked.push(found.ok_or_else(|| format!("未登记镜像 {image}"))?);
    }
    Ok(picked)
}

struct BuildGroup<'a> {
    manifest: &'static str,
    package: &'static str,
    target: &'static str,
    profile: &'static str,
    features: &'static [&'static str],
    default_features: bool,
    builds: Vec<&'a ImageBuild>,
}

fn groups<'a>(picked: &[&'a ImageBuild]) -> Vec<BuildGroup<'a>> {
    let mut groups: Vec<BuildGroup<'a>> = Vec::new();
    for build in picked {
        let existing = groups.iter().position(|group| {
            group.manifest == build.manifest
                && group.package == build.package
                && group.target == build.target
                && group.profile == build.profile
                && group.features == build.features
                && group.default_features == build.default_features
        });
        match existing {
            Some(index) => groups[index].builds.push(build),
            None => groups.push(BuildGroup {
                manifest: build.manifest,
                package: build.package,
                target: build.target,
                profile: build.profile,
                features: build.features,
                default_features: build.default_features,
                builds: vec![build],
            }),
        }
    }
    groups
}

fn build_group(
    root: &Path,
    work: &Path,
    group: &BuildGroup<'_>,
) -> Result<Vec<(&'static str, PathBuf)>, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let manifest = root.join(group.manifest);
    let expected = package_id(&cargo, &manifest, group.package)?;
    let mut args = vec![
        "build".to_string(),
        "--locked".to_string(),
        "--manifest-path".to_string(),
        manifest.to_string_lossy().into_owned(),
        "-p".to_string(),
        group.package.to_string(),
        "--message-format=json".to_string(),
        "--target".to_string(),
        group.target.to_string(),
        "--target-dir".to_string(),
        work.to_string_lossy().into_owned(),
        "--profile".to_string(),
        group.profile.to_string(),
    ];
    for build in &group.builds {
        args.push("--bin".to_string());
        args.push(build.bin.to_string());
    }
    if !group.features.is_empty() {
        args.push("--features".to_string());
        args.push(group.features.join(","));
    }
    if !group.default_features {
        args.push("--no-default-features".to_string());
    }
    let output = Command::new(&cargo)
        .args(&args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("起不动嵌套 cargo：{e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: String = stderr.chars().rev().take(4000).collect::<String>().chars().rev().collect();
        return Err(format!(
            "{} 编不过（{}）：\n{tail}",
            group.package,
            output.status
        ));
    }

    let mut found = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value["reason"] != "compiler-artifact"
            || value["package_id"] != expected.as_str()
            || !contains(&value["target"]["kind"], "bin")
            || value["profile"]["test"].as_bool().unwrap_or(true)
        {
            continue;
        }
        let Some(build) = group
            .builds
            .iter()
            .find(|build| value["target"]["name"] == build.bin)
        else {
            continue;
        };
        let Some(executable) = value["executable"].as_str() else {
            return Err(format!("镜像 {} 的可执行产物缺失", build.image));
        };
        println!(
            "image resolved: image={} package={} bin={} target={} profile={} features={:?} executable={}",
            build.image,
            value["package_id"].as_str().unwrap_or(""),
            build.bin,
            group.target,
            group.profile,
            value["features"],
            executable
        );
        found.push((build.image, PathBuf::from(executable)));
    }

    let mut artifacts = Vec::new();
    for build in &group.builds {
        let matches: Vec<_> = found
            .iter()
            .filter(|(image, _)| *image == build.image)
            .collect();
        match matches.as_slice() {
            [] => return Err(format!("Cargo 缺少镜像 {} 的执行产物", build.image)),
            [single] => artifacts.push((build.image, checked_elf(&single.1)?)),
            _ => return Err(format!("Cargo 对镜像 {} 报出多个执行产物", build.image)),
        }
    }
    Ok(artifacts)
}

fn package_id(cargo: &str, manifest: &Path, package: &str) -> Result<String, String> {
    let output = Command::new(cargo)
        .args([
            "metadata",
            "--locked",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()
        .map_err(|e| format!("起不动 cargo metadata：{e}"))?;
    if !output.status.success() {
        return Err("cargo metadata 失败".to_string());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|e| format!("cargo metadata JSON：{e}"))?;
    value["packages"]
        .as_array()
        .and_then(|packages| {
            packages
                .iter()
                .find(|item| item["name"] == package)
                .and_then(|item| item["id"].as_str())
        })
        .map(str::to_string)
        .ok_or_else(|| format!("cargo metadata 缺少包 {package}"))
}

fn contains(value: &serde_json::Value, want: &str) -> bool {
    value
        .as_array()
        .is_some_and(|items| items.iter().any(|item| item == want))
}

fn checked_elf(path: &Path) -> Result<PathBuf, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|e| format!("执行产物 {} 不可读：{e}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("执行产物 {} 不是文件", path.display()));
    }
    let mut file = std::fs::File::open(path)
        .map_err(|e| format!("执行产物 {} 打不开：{e}", path.display()))?;
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)
        .map_err(|e| format!("执行产物 {} 读不了：{e}", path.display()))?;
    if magic != *b"\x7fELF" {
        return Err(format!("执行产物 {} 不是 ELF", path.display()));
    }
    Ok(path.to_path_buf())
}

/// 瘦一份程序镜像：剥掉符号与调试节，只留可装载内容（见文件头"瘦身"那段）。
///
/// `src` is the Cargo executable; it stays unchanged for host debugging. `out` is the work directory.
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
    use super::{ImageBuild, bins_with, checked_elf, needed_builds, order_of, validate_builds};

    /// Test-only projection of the in-repo composition values. Normal builds of
    /// this crate never depend on composition.
    mod unit {
        pub use ::unit::*;
        pub use composition::{DEPLOYMENTS, IMAGE_BUILDS, PROGRAMS};
    }

    fn deployment_of(scene: &str) -> Option<&'static unit::Deployment> {
        unit::Deployment::find(unit::DEPLOYMENTS, scene)
    }

    fn bins_for(
        deployment: &'static unit::Deployment,
    ) -> Result<Vec<(&'static str, env::ProgramKind)>, String> {
        bins_with(deployment, unit::PROGRAMS, unit::IMAGE_BUILDS)
    }

    const fn fixture_build(image: &'static str, bin: &'static str) -> ImageBuild {
        ImageBuild {
            image,
            manifest: "unused/Cargo.toml",
            package: "unused",
            bin,
            features: &[],
            target: "riscv64gc-unknown-none-elf",
            profile: "dev",
            default_features: true,
        }
    }

    fn select(
        name: &str,
        entry: env::ledger::manifest::Entry<'_>,
    ) -> Result<&'static unit::Deployment, unit::DeploymentSelectionError> {
        unit::select_deployment(unit::DEPLOYMENTS, unit::PROGRAMS, name, entry)
    }

    #[test]
    fn deployment_image_and_start_members_are_registered() {
        for deployment in unit::DEPLOYMENTS {
            let images = bins_for(deployment).unwrap();
            order_of(deployment, unit::PROGRAMS).unwrap();
            let entry_image = deployment.entry_image(unit::PROGRAMS).unwrap();
            assert!(images.iter().any(|(name, _)| *name == entry_image));
            for (image, _) in &images {
                assert!(unit::IMAGE_BUILDS.iter().any(|build| build.image == *image));
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
        let fault = deployment_of("system-fault").unwrap();
        let fault_bins = bins_for(fault).unwrap();
        for required in ["shell", "cat", "emit", "upper", "fail", "spin", "workers"] {
            assert!(fault_bins.iter().any(|(image, _)| *image == required));
        }
        assert!(!fault.start.contains(&"shell"));
        assert!(!order_of(fault, unit::PROGRAMS).unwrap().contains(&"shell"));

        let product = bins_for(deployment_of("product").unwrap())
            .unwrap()
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>();
        for supplied in ["shell", "cat", "emit", "upper", "fail", "spin", "workers"] {
            assert!(product.contains(&supplied));
        }
    }

    #[test]
    fn daemon_is_one_rtc_only_supervised_process_in_product_scenes() {
        const SCENES: &[&str] = &[
            "product",
            "iommu-product",
            "iommu-product-fault",
            "iommu-product-before",
            "iommu-product-after",
            "daemon-rtc",
            "daemon-target",
            "daemon-stop",
        ];
        for deployment in unit::DEPLOYMENTS {
            let count = deployment
                .start
                .iter()
                .filter(|name| **name == "daemon")
                .count();
            assert_eq!(
                count,
                usize::from(SCENES.contains(&deployment.name)),
                "daemon count in {}",
                deployment.name
            );
            let supervised = deployment
                .targets
                .iter()
                .find(|target| target.name == "supervised");
            assert_eq!(
                supervised.is_some(),
                SCENES.contains(&deployment.name),
                "supervised group presence in {}",
                deployment.name
            );
        }
        for scene in SCENES {
            let deployment = deployment_of(scene).unwrap();
            let order = order_of(deployment, unit::PROGRAMS).unwrap();
            let daemon = order.iter().position(|name| *name == "daemon").unwrap();
            let provider = if matches!(*scene, "daemon-target" | "daemon-stop") {
                "fixturetarget"
            } else {
                "rtc"
            };
            let provider_at = order.iter().position(|name| *name == provider).unwrap();
            assert!(
                daemon > provider_at,
                "daemon must start after {provider} in {scene}"
            );
            let supervised = deployment
                .targets
                .iter()
                .find(|target| target.name == "supervised")
                .unwrap();
            assert_eq!(
                supervised.members,
                [provider],
                "supervised group in {scene}"
            );
            assert!(
                supervised
                    .members
                    .iter()
                    .all(|member| !deployment.guarded.contains(member))
            );
            assert!(
                deployment
                    .targets
                    .iter()
                    .any(|target| target.name == unit::SCENE
                        && target.members.contains(&"daemon")),
                "daemon missing from {scene} scene target"
            );
            assert!(
                bins_for(deployment)
                    .unwrap()
                    .iter()
                    .any(|(image, _)| *image == "daemon")
            );
            assert!(
                bins_for(deployment)
                    .unwrap()
                    .iter()
                    .any(|(image, _)| *image == provider)
            );
        }
    }

    #[test]
    fn package_names_are_unique_and_exclude_scene_targets() {
        for (index, build) in unit::IMAGE_BUILDS.iter().enumerate() {
            assert!(!build.image.is_empty());
            assert!(build.bin.starts_with("prog-"));
            assert!(unit::IMAGE_BUILDS[..index].iter().all(|earlier| {
                earlier.image != build.image && earlier.bin != build.bin
            }));
        }
        assert!(
            !unit::IMAGE_BUILDS
                .iter()
                .any(|build| build.image == "scene")
        );
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
                supervision: None,
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
                supervision: None,
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
                supervision: None,
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
                supervision: None,
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
                supervision: None,
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
        static BUILDS: &[ImageBuild] = &[
            fixture_build("shared-image", "shared-elf"),
            fixture_build("unitless-image", "raw-elf"),
            fixture_build("supplied-image", "supplied-elf"),
        ];

        let bins = bins_with(&DEPLOYMENT, &[&ENTRY, &FIRST, &SECOND], BUILDS).unwrap();
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
                supervision: None,
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
        static BUILDS: &[ImageBuild] = &[fixture_build("unknown-entry", "unknown-entry-elf")];

        let error = bins_with(&DEPLOYMENT, &[&ENTRY], BUILDS).unwrap_err();
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
                supervision: None,
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
        static BUILDS: &[ImageBuild] = &[fixture_build("raw-entry", "raw-entry-elf")];

        assert_eq!(
            bins_with(&DEPLOYMENT, &[&ENTRY], BUILDS).unwrap(),
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
                supervision: None,
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
                supervision: None,
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
                supervision: None,
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
                select(name, entry),
                Err(unit::DeploymentSelectionError::Unknown)
            ));
        }
        assert!(matches!(
            select("accept", entry),
            Err(unit::DeploymentSelectionError::EntryMismatch)
        ));
        assert!(matches!(
            select("product", env::ledger::manifest::Entry {
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
                deployment.guarded(unit::PROGRAMS).is_ok(),
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
                .position(|(name, _)| Some(*name) == deployment.entry_image(unit::PROGRAMS)).unwrap();
            let blob = env::ledger::manifest::pack(&items, entry_at, deployment.name).unwrap();
            let entries = env::ledger::manifest::Entries::new(&blob).unwrap();
            let selected = select(entries.deployment(), entries.entry().unwrap()).unwrap();
            assert_eq!(unit::DEPLOYMENTS.iter().filter(|other| other.name == deployment.name).count(), 1);
            assert_eq!(selected.name, deployment.name);
            assert_eq!(selected.entry, deployment.entry);
            assert_eq!(selected.start, deployment.start);
            assert_eq!(entries.count(), items.len());
        }
    }

    #[test]
    fn long_soak_uses_loader_arguments_without_another_controller_image() {
        let deployment = deployment_of("iommu-soak-long").unwrap();
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
            let deployment = deployment_of(name).unwrap();
            assert_eq!(deployment.entry, "system");
            assert_eq!(deployment.entry_image(unit::PROGRAMS), Some("system"));
            let bins = bins_for(deployment).unwrap();
            assert_eq!(bins.iter().filter(|(name, _)| *name == "system").count(), 1);
            assert!(!bins.iter().any(|(name, _)| name.starts_with("system-iommu")));
            for candidate in ["device-ctl", "device-ctl-fault", "device-ctl-before", "device-ctl-after"] {
                assert_eq!(deployment.start.contains(&candidate), controller == Some(candidate));
            }
        }
        assert!(!unit::IMAGE_BUILDS.iter().any(|build| build.image.starts_with("system-iommu")));
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
            select(entries.deployment(), entries.entry().unwrap()).unwrap().name,
            "iommu-product-fault",
        );
        let wrong = pack(items, 0, "iommu-product-fault").unwrap();
        let entries = Entries::new(&wrong).unwrap();
        assert!(matches!(
            select(entries.deployment(), entries.entry().unwrap()),
            Err(unit::DeploymentSelectionError::EntryMismatch)
        ));
    }

    #[test]
    fn image_mapping_rejects_duplicate_and_missing_entries() {
        let duplicate = [
            fixture_build("same", "one"),
            fixture_build("same", "two"),
        ];
        assert!(validate_builds(&duplicate).unwrap_err().contains("重复镜像"));
        let bins = [("missing", env::ProgramKind::User)];
        assert!(
            needed_builds(&bins, &[fixture_build("present", "bin")])
                .err()
                .expect("missing image must fail")
                .contains("未登记镜像")
        );
    }

    #[test]
    fn non_elf_artifact_is_rejected() {
        let at = std::env::temp_dir().join(format!(
            "shuyra-image-non-elf-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&at, b"not an elf").unwrap();
        let error = checked_elf(&at).unwrap_err();
        let _ = std::fs::remove_file(&at);
        assert!(error.contains("不是 ELF"), "{error}");
    }
}
