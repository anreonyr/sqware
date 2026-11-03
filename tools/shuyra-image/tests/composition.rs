//! Actual Source composition assertions against the published image-builder
//! planning API. The provider core knows only explicit declarations, so these
//! stay in Integration; the three catalog cases are in `catalog.rs`.

use composition::units::pipe;
use composition::{DEPLOYMENTS, IMAGE_BUILDS, PROGRAMS};
use env::ProgramKind;
use image::{image_inventory, startup_order};
use unit::Deployment;

fn deployment_of(scene: &str) -> Option<&'static Deployment> {
    Deployment::find(DEPLOYMENTS, scene)
}

fn bins_for(deployment: &'static Deployment) -> Result<Vec<(&'static str, ProgramKind)>, String> {
    image_inventory(PROGRAMS, deployment, IMAGE_BUILDS)
}

fn select(
    name: &str,
    entry: env::ledger::manifest::Entry<'_>,
) -> Result<&'static Deployment, unit::DeploymentSelectionError> {
    unit::select_deployment(DEPLOYMENTS, PROGRAMS, name, entry)
}

#[test]
fn deployment_image_and_start_members_are_registered() {
    for deployment in DEPLOYMENTS {
        let images = bins_for(deployment).unwrap();
        startup_order(PROGRAMS, deployment).unwrap();
        let entry_image = deployment.entry_image(PROGRAMS).unwrap();
        assert!(images.iter().any(|(name, _)| *name == entry_image));
        for (image, _) in &images {
            assert!(IMAGE_BUILDS.iter().any(|build| build.image == *image));
        }
        for start in deployment.start {
            let program = PROGRAMS
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
    assert!(!startup_order(PROGRAMS, fault).unwrap().contains(&"shell"));

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
    for deployment in DEPLOYMENTS {
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
        let order = startup_order(PROGRAMS, deployment).unwrap();
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
    assert_eq!(IMAGE_BUILDS.len(), 89);
    for (index, build) in IMAGE_BUILDS.iter().enumerate() {
        assert!(!build.image.is_empty());
        if build.image == "pipe" {
            assert_eq!(build.image, pipe::BUILD.image);
            assert_eq!(build.manifest, "components/pipe/server/Cargo.toml");
            assert_eq!(build.package, pipe::BUILD.package);
            assert_eq!(build.bin, pipe::BUILD.bin);
            assert_eq!(build.features, pipe::BUILD.features);
            assert_eq!(build.target, pipe::BUILD.target);
            assert_eq!(build.profile, pipe::BUILD.profile);
            assert_eq!(build.default_features, pipe::BUILD.default_features);
        } else {
            assert_eq!(build.manifest, "programs/Cargo.toml");
            assert_eq!(build.package, "programs");
            assert!(build.bin.starts_with("prog-"));
            assert_eq!(build.bin, "prog-".to_owned() + build.image);
            assert_ne!(build.bin, "pipe");
        }
        assert!(IMAGE_BUILDS[..index].iter().all(|earlier| {
            earlier.image != build.image && earlier.bin != build.bin
        }));
    }
    assert!(!IMAGE_BUILDS.iter().any(|build| build.image == "scene"));
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
        select(
            "product",
            env::ledger::manifest::Entry {
                kind: env::ProgramKind::User,
                ..entry
            }
        ),
        Err(unit::DeploymentSelectionError::EntryMismatch)
    ));
}

#[test]
fn guarded_deployments_name_only_started_catalog_programs() {
    for deployment in DEPLOYMENTS {
        assert!(
            deployment.guarded(PROGRAMS).is_ok(),
            "deployment {} has an invalid guard selection",
            deployment.name
        );
    }
}

#[test]
fn packaged_selection_roundtrips_for_every_deployment() {
    for deployment in DEPLOYMENTS {
        let images = bins_for(deployment).unwrap();
        let items: Vec<_> = images
            .iter()
            .map(|(name, kind)| (*kind, *name, b"actual image placeholder".as_slice()))
            .collect();
        let entry_at = images
            .iter()
            .position(|(name, _)| Some(*name) == deployment.entry_image(PROGRAMS))
            .unwrap();
        let blob = env::ledger::manifest::pack(&items, entry_at, deployment.name).unwrap();
        let entries = env::ledger::manifest::Entries::new(&blob).unwrap();
        let selected = select(entries.deployment(), entries.entry().unwrap()).unwrap();
        assert_eq!(
            DEPLOYMENTS
                .iter()
                .filter(|other| other.name == deployment.name)
                .count(),
            1
        );
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
    let controller = PROGRAMS
        .iter()
        .find(|program| program.name() == "device-ctl-soak-long")
        .unwrap();
    assert_eq!(controller.loader.image, "device-ctl-soak");
    assert_eq!(controller.loader.args, &[1024]);
    let bins = bins_for(deployment).unwrap();
    assert_eq!(
        bins.iter()
            .filter(|(name, _)| *name == "device-ctl-soak")
            .count(),
        1
    );
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
        assert_eq!(deployment.entry_image(PROGRAMS), Some("system"));
        let bins = bins_for(deployment).unwrap();
        assert_eq!(bins.iter().filter(|(name, _)| *name == "system").count(), 1);
        assert!(!bins.iter().any(|(name, _)| name.starts_with("system-iommu")));
        for candidate in [
            "device-ctl",
            "device-ctl-fault",
            "device-ctl-before",
            "device-ctl-after",
        ] {
            assert_eq!(
                deployment.start.contains(&candidate),
                controller == Some(candidate)
            );
        }
    }
    assert!(!IMAGE_BUILDS
        .iter()
        .any(|build| build.image.starts_with("system-iommu")));
    assert!(!PROGRAMS
        .iter()
        .any(|program| program.name().starts_with("system-iommu")));
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
        select(entries.deployment(), entries.entry().unwrap())
            .unwrap()
            .name,
        "iommu-product-fault",
    );
    let wrong = pack(items, 0, "iommu-product-fault").unwrap();
    let entries = Entries::new(&wrong).unwrap();
    assert!(matches!(
        select(entries.deployment(), entries.entry().unwrap()),
        Err(unit::DeploymentSelectionError::EntryMismatch)
    ));
}
