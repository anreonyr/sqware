use std::path::{Path, PathBuf};

use image::{ImageBuild, build_with, executables};
use unit::{ControlConfig, Deployment, Identity, LoaderConfig, OperatorConfig, SupplyConfig, UnitFile};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/link-workspace")
        .canonicalize()
        .unwrap()
}

fn build(
    image: &'static str,
    package: &'static str,
    bin: &'static str,
    manifest: &'static str,
    features: &'static [&'static str],
    profile: &'static str,
) -> ImageBuild {
    ImageBuild {
        image,
        manifest,
        package,
        bin,
        features,
        target: "riscv64gc-unknown-none-elf",
        profile,
        default_features: true,
    }
}

fn assert_riscv_elf(path: &Path) {
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..4], b"\x7fELF");
    assert_eq!(u16::from_le_bytes([bytes[16], bytes[17]]), 2, "ET_EXEC");
    assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 243, "EM_RISCV");
    assert_ne!(
        u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
        0,
        "zero entry"
    );
}

#[test]
fn second_package_and_bin_are_selected_from_real_cargo_artifacts() {
    let root = fixture();
    let work = Path::new(env!("CARGO_TARGET_TMPDIR")).join("image-second-package");
    let builds = [
        build(
            "first-image",
            "link-first",
            "first",
            "first/Cargo.toml",
            &[],
            "dev",
        ),
        build(
            "second-image",
            "link-second",
            "second",
            "second/Cargo.toml",
            &[],
            "dev",
        ),
    ];
    let artifacts = executables(&root, &work, &builds).unwrap();
    assert_eq!(artifacts.len(), 2);
    for (image, path) in &artifacts {
        assert!(path.is_file(), "{image}: {}", path.display());
        assert!(path.ends_with(if *image == "first-image" {
            "first"
        } else {
            "second"
        }));
        assert_riscv_elf(path);
    }
    assert_ne!(artifacts[0].1, artifacts[1].1);
    assert_ne!(
        std::fs::read(&artifacts[0].1).unwrap(),
        std::fs::read(&artifacts[1].1).unwrap()
    );
}

#[test]
fn release_with_debug_assertions_and_dependency_feature_request() {
    let root = fixture();
    let work = Path::new(env!("CARGO_TARGET_TMPDIR")).join("image-release");
    let builds = [build(
        "second-image",
        "link-second",
        "second",
        "second/Cargo.toml",
        &["link-dep/loud"],
        "release",
    )];
    let artifacts = executables(&root, &work, &builds).unwrap();
    assert_eq!(artifacts.len(), 1);
    assert!(artifacts[0].1.is_file());
    assert_riscv_elf(&artifacts[0].1);
}

fn has(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

#[test]
fn same_bin_with_different_features_keeps_both_real_el_artifacts() {
    let root = fixture();
    let work = Path::new(env!("CARGO_TARGET_TMPDIR")).join("image-feature-variant");
    let builds = [
        build(
            "variant-plain",
            "link-second",
            "second",
            "second/Cargo.toml",
            &[],
            "dev",
        ),
        build(
            "variant-loud",
            "link-second",
            "second",
            "second/Cargo.toml",
            &["link-dep/loud"],
            "dev",
        ),
    ];
    let artifacts = executables(&root, &work, &builds).unwrap();
    assert_eq!(artifacts.len(), 2);
    let plain = artifacts
        .iter()
        .find(|(image, _)| *image == "variant-plain")
        .map(|(_, path)| path)
        .unwrap();
    let loud = artifacts
        .iter()
        .find(|(image, _)| *image == "variant-loud")
        .map(|(_, path)| path)
        .unwrap();
    let plain = std::fs::read(plain).unwrap();
    let loud = std::fs::read(loud).unwrap();
    assert!(has(&plain, b"link-dep-plain-note"));
    assert!(!has(&plain, b"link-dep-loud-note"));
    assert!(has(&loud, b"link-dep-loud-note"));
    assert!(!has(&loud, b"link-dep-plain-note"));
    assert_ne!(plain, loud);
}

static ENTRY_PROGRAM: UnitFile = UnitFile {
    name: "entry",
    loader: LoaderConfig::supervisor("first-image"),
    operator: OperatorConfig::DEFAULT,
    identity: Identity::DEFAULT,
    control: ControlConfig::DEFAULT,
    supply: SupplyConfig::DEFAULT,
};

static VARIANT_DEPLOYMENT: Deployment = Deployment {
    name: "variant-pack",
    entry: "entry",
    start: &[],
    guarded: &[],
    resources: &["variant-plain", "variant-loud"],
    targets: &[],
    bootstrap: &[],
    sources: &[],
};

#[test]
fn build_with_packs_each_feature_variant_from_its_own_artifact() {
    let root = fixture();
    let work = Path::new(env!("CARGO_TARGET_TMPDIR")).join("image-pack-variant");
    let output = Path::new(env!("CARGO_TARGET_TMPDIR")).join("variant-initrd.img");
    let builds = [
        build("first-image", "link-first", "first", "first/Cargo.toml", &[], "dev"),
        build(
            "variant-plain",
            "link-second",
            "second",
            "second/Cargo.toml",
            &[],
            "dev",
        ),
        build(
            "variant-loud",
            "link-second",
            "second",
            "second/Cargo.toml",
            &["link-dep/loud"],
            "dev",
        ),
    ];
    let at = build_with(
        &root,
        &work,
        &output,
        &[&ENTRY_PROGRAM],
        &VARIANT_DEPLOYMENT,
        &builds,
    )
    .unwrap();
    let blob = std::fs::read(at).unwrap();
    let entries = env::ledger::manifest::Entries::new(&blob).unwrap();
    let mut plain = None;
    let mut loud = None;
    for entry in entries {
        let entry = entry.unwrap();
        match entry.name {
            "variant-plain" => plain = Some(entry.elf),
            "variant-loud" => loud = Some(entry.elf),
            _ => {}
        }
    }
    let plain = plain.expect("plain manifest entry");
    let loud = loud.expect("loud manifest entry");
    assert!(has(plain, b"link-dep-plain-note"));
    assert!(!has(plain, b"link-dep-loud-note"));
    assert!(has(loud, b"link-dep-loud-note"));
    assert!(!has(loud, b"link-dep-plain-note"));
}
