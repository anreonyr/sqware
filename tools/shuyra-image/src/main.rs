//! `shuyra-image [场景] [档]` — Integration adapter for the image-builder core.

use std::path::Path;

use unit::{Deployment, ImageBuild};

fn profile(value: &str) -> (&str, &'static str) {
    match value {
        "debug" => ("debug", "dev"),
        "release" => ("release", "release"),
        other => (other, Box::leak(other.to_string().into_boxed_str())),
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let scenario = args.next().unwrap_or_else(|| "product".to_string());
    let profile_arg = args.next().unwrap_or_else(|| "release".to_string());
    if let Some(extra) = args.next() {
        eprintln!("image: 多出来的参数 `{extra}`（用法：shuyra-image [场景] [档]）");
        std::process::exit(2);
    }
    let (label, cargo_profile) = profile(&profile_arg);
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = root.join("../..");
    let root = root.canonicalize().unwrap_or(root);
    let Some(deployment) = Deployment::find(composition::DEPLOYMENTS, &scenario) else {
        eprintln!(
            "image: 不认得的景 {scenario}（认得的：{}）",
            composition::DEPLOYMENTS
                .iter()
                .map(|deployment| deployment.name)
                .collect::<Vec<_>>()
                .join(" / ")
        );
        std::process::exit(2);
    };
    let builds: Vec<ImageBuild> = composition::IMAGE_BUILDS
        .iter()
        .map(|build| ImageBuild {
            profile: cargo_profile,
            ..*build
        })
        .collect();
    let Some(target) = builds.first().map(|build| build.target) else {
        eprintln!("image: 构建映射为空");
        std::process::exit(1);
    };
    let work = root.join("target/image").join(label);
    let output = root
        .join("target")
        .join(target)
        .join(label)
        .join("initrd.img");
    match image::build_with(
        &root,
        &work,
        &output,
        composition::PROGRAMS,
        deployment,
        &builds,
    ) {
        Ok(at) => println!("ok  {}", at.display()),
        Err(why) => {
            eprintln!("image: {why}");
            std::process::exit(1);
        }
    }
}
