#[cfg(test)]
mod boundaries {
    use std::{fs, path::Path};
    use syn::visit::{self, Visit};
    #[derive(Default)]
    struct References(Vec<String>);
    impl<'a> Visit<'a> for References {
        fn visit_path(&mut self, path: &'a syn::Path) {
            self.0.push(path.segments.iter().map(|p| p.ident.to_string()).collect::<Vec<_>>().join("::"));
            visit::visit_path(self, path);
        }
        fn visit_item_use(&mut self, item: &'a syn::ItemUse) {
            fn collect(tree: &syn::UseTree, prefix: &str, out: &mut Vec<String>) {
                match tree {
                    syn::UseTree::Path(path) => collect(&path.tree, &format!("{prefix}{}::", path.ident), out),
                    syn::UseTree::Group(group) => for tree in &group.items { collect(tree, prefix, out); },
                    syn::UseTree::Name(name) => out.push(format!("{prefix}{}", name.ident)),
                    syn::UseTree::Rename(name) => out.push(format!("{prefix}{}", name.ident)),
                    syn::UseTree::Glob(_) => out.push(prefix.trim_end_matches("::").to_owned()),
                }
            }
            collect(&item.tree, "", &mut self.0);
        }
    }
    fn references(root: &Path, out: &mut References) {
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() { references(&path, out); }
            else if path.extension().is_some_and(|ext| ext == "rs") {
                out.visit_file(&syn::parse_file(&fs::read_to_string(path).unwrap()).unwrap());
            }
        }
    }
    #[test]
    fn driver_retention_uses_the_same_supply_acquisition_as_service_installation() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src");
        let hub = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../components/driver/units/hub/server/src");
        let iommu_dir = src.join("driver/iommu");
        let mut driver_files: Vec<String> = fs::read_dir(&iommu_dir).unwrap().filter_map(|entry| {
            let path = entry.unwrap().path();
            (path.extension().is_some_and(|ext| ext == "rs")).then(|| path.file_name().unwrap().to_string_lossy().into_owned())
        }).collect();
        driver_files.sort();
        let mut expected = vec!["acquire.rs","binding.rs","cache_step.rs","context.rs","cookie.rs","core.rs","domain.rs","domain_id.rs","domain_range.rs","engine.rs","fence.rs","handshake.rs","invalidation.rs","layout.rs","main.rs","mod.rs","order.rs","pages.rs","phase.rs","queue.rs","references.rs","regs.rs","serve.rs","settled.rs","table.rs","ticket.rs"];
        expected.sort();
        assert_eq!(driver_files, expected, "the production IOMMU keeps exactly one bin and no scene variants");
        let core = fs::read_to_string(iommu_dir.join("core.rs")).unwrap();
        for step in ["pub fn establish","pub fn acquire","pub fn publish","pub fn invalidate","pub fn quiesce","pub fn release"] { assert!(core.contains(step), "missing step {step}"); }
        let domain = fs::read_to_string(iommu_dir.join("domain.rs")).unwrap();
        assert!(domain.contains("Holds::new()"));
        assert!(core.contains("Domain::new("));
        let compact: String = core.chars().filter(|ch| !ch.is_whitespace()).collect();
        assert!(compact.contains("holds.register("));
        assert!(compact.contains("holds.retry("));
        assert!(!core.contains("raw::pies("));
        assert!(!core.contains("hub_api::iova_base"));
        let work = fs::read_to_string(src.join("harness/device/work.rs")).unwrap();
        assert_eq!(work.matches("raw::pies(").count(), 1, "only the Before fixture may scan local gifts");
        let before = work.find("if mode == Mode::Before").unwrap();
        let window = work.find("let window = await_window").unwrap();
        assert!(work[before..window].contains("await_bar_gift("));
        assert_eq!(work.matches("await_bar_gift(").count(), 2, "gift verification is defined and called once");
        let gift = work.find("fn await_bar_gift(").unwrap();
        let gift_end = work[gift + 1..].find("\nfn ").map_or(work.len(), |end| gift + 1 + end);
        assert!(work[gift..gift_end].contains("raw::pies("), "the local scan belongs to the Before fixture's gift verifier");
        assert!(!work.contains("find_window("));
        assert!(!work.contains("find_bar("));
        let control = fs::read_to_string(src.join("harness/device/control.rs")).unwrap();
        assert!(!control.contains("iova_base("));
        let bindings = fs::read_to_string(hub.join("core/bindings.rs")).unwrap();
        assert!(bindings.contains("pub fn owner_gone"));
        assert!(bindings.contains("Fail::Taken"));
        assert!(!bindings.contains("iova_base"));
        assert!(!bindings.contains("Cell::Vacant"));
        let dma = fs::read_to_string(hub.join("serve/dma.rs")).unwrap();
        assert!(!dma.contains("join_task("));
        assert!(!dma.contains("slay_team("));
        assert!(!dma.contains("env::unit::oust("));
        let hub_core = hub.join("core");
        for old in ["execution.rs","execution_effects.rs","execution_failure.rs","execution_step.rs","executions.rs"] { assert!(!hub_core.join(old).exists(), "Hub lifecycle state remains: {old}"); }
        assert!(hub_core.join("admission.rs").is_file());
        let hub_execution = fs::read_to_string(hub.join("serve/execution.rs")).unwrap();
        assert!(!hub_execution.contains("env::unit::"));
        assert!(fs::read_to_string(hub.join("serve/launch.rs")).unwrap().contains("begin_inspect"));
        let hub_return = fs::read_to_string(hub.join("serve/returned.rs")).unwrap();
        assert!(hub_return.contains("begin_release"));
        assert!(hub.join("serve/launch_phase.rs").is_file());
        assert!(hub.join("serve/launch_reply.rs").is_file());
        assert!(hub.join("serve/return_reply.rs").is_file());
        assert!(!dma.contains("fn stop_cpu"));
        assert!(!dma.contains("owned_instance"));
        assert!(!hub_return.contains("Err(Fail::Unknown) | Err(Fail::Denied) => ReturnReply::Released"));
        assert!(hub_execution.contains("try_reserve"));
        assert!(dma.contains("device_by_door("));
        assert!(!dma.contains("claim.requester"));
        assert!(dma.contains("peers.coord.host() != from"));
        assert!(!dma.contains("crate::driver::pci::"));
        let coordination = fs::read_to_string(hub.join("serve/coordination.rs")).unwrap();
        assert!(coordination.contains("binding.device.source != peers.coord.host()"));
    }
    #[test]
    fn instance_retirement_precedes_cleanup_and_dynamic_units_declare_hub() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let device = fs::read_to_string(repo.join("crates/composition/src/units/device.rs")).unwrap();
        assert!(device.contains("retirement: &WORK_RETIREMENT"));
        assert!(device.contains("provider: \"hub\""));
        for path in ["programs/src/user/login/main.rs","programs/src/user/shell/adapt/controller.rs"] {
            let source = fs::read_to_string(repo.join(path)).unwrap();
            assert!(!source.contains("env::unit::oust"), "{path} still Ousts directly");
            assert!(source.contains("release"), "{path} does not return through Release");
        }
        let shell = fs::read_to_string(repo.join("programs/src/user/shell/adapt/controller.rs")).unwrap();
        assert!(shell.contains("begin_release"));
        assert!(shell.contains("release_reply"));
        assert!(!shell.contains("env::unit::oust"));
    }
    #[test]
    fn ordinary_programs_do_not_depend_on_system_implementation() {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src");
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        for (area, root) in [
            ("driver", source.join("driver")),
            ("service", source.join("service")),
            ("user", source.join("user")),
            ("selected-hub-client", repo.join("components/driver/units/hub/client/src")),
            ("selected-hub-server", repo.join("components/driver/units/hub/server/src")),
            ("selected-router-api", repo.join("components/driver/units/router/api/src")),
            ("selected-router-client", repo.join("components/driver/units/router/client/src")),
            ("selected-router-server", repo.join("components/driver/units/router/server/src")),
            ("selected-driver-shared", repo.join("components/driver/shared/src")),
            ("selected-driver-support", repo.join("components/driver/support/src")),
        ] {
            let mut paths = References::default();
            references(&root, &mut paths);
            assert!(!paths.0.iter().any(|path| path.starts_with("crate::system::") || path.starts_with("programs::system::") || path.starts_with("system_server::")), "{area} enters System implementation: {:?}", paths.0);
            if area.starts_with("selected-") {
                assert!(!paths.0.iter().any(|path| path.starts_with("programs::") || path.starts_with("composition::")), "{area} links back to Source: {:?}", paths.0);
            }
        }
    }
    #[test]
    fn mark_declarations_cannot_bypass_the_registry() {
        fn expression<'a>(syntax: &'a syn::File, name: &str) -> &'a syn::Expr {
            syntax.items.iter().find_map(|item| match item { syn::Item::Const(item) if item.ident == name => Some(&*item.expr), _ => None }).unwrap_or_else(|| panic!("missing const {name}"))
        }
        #[derive(Default)]
        struct Paths(Vec<String>);
        impl<'ast> Visit<'ast> for Paths {
            fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
                self.0.push(path.path.segments.iter().map(|p| p.ident.to_string()).collect::<Vec<_>>().join("::"));
                visit::visit_expr_path(self, path);
            }
        }
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let assembly = syn::parse_file(&fs::read_to_string(repo.join("programs/src/interfaces.rs")).unwrap()).unwrap();
        let mut registered = Paths::default(); registered.visit_expr(expression(&assembly, "APIS"));
        for provider in ["LOADER","system_api::identity::REGISTRY","system_api::operator::REGISTRY","system_api::control::REGISTRY","hub_api::REGISTRY","terminal_api::REGISTRY","router_api::REGISTRY"] {
            assert!(registered.0.iter().any(|path| path == provider), "unit interface registry omits provider {provider}");
        }
        let mut loader = Paths::default(); loader.visit_expr(expression(&assembly, "LOADER"));
        assert!(loader.0.iter().any(|path| path == "system_api::loader::REGISTRY"));
        for api in ["components/driver/units/router/api/src/lib.rs"] {
            assert!(fs::read_to_string(repo.join(api)).unwrap().contains("mold::interface"));
        }
    }
    #[test]
    fn provider_api_and_public_clients_keep_one_way_dependencies() {
        fn dependencies(path: &Path) -> Vec<String> {
            let source = fs::read_to_string(path).unwrap();
            let mut result = Vec::new(); let mut section = false;
            for line in source.lines().map(str::trim) {
                if line.starts_with('[') { section = line.contains("dependencies"); }
                else if section { if let Some((name, _)) = line.split_once('=') { result.push(name.trim().to_owned()); } }
            }
            result
        }
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let programs = fs::read_to_string(repo.join("programs/Cargo.toml")).unwrap();
        for name in ["terminal-api", "terminal-client"] {
            let line = programs.lines().find(|line| line.trim_start().starts_with(name)).unwrap();
            assert!(line.contains("https://github.com/shuyra-arche/terminal.git"));
            assert!(line.contains("rev = \"c447c0761835339a39efdf2ddd8101ebf80db15a\""));
        }
        assert!(!programs.contains("src/user/terminal/api"));
        assert!(!programs.contains("src/user/terminal/client"));
        let router_api = dependencies(&repo.join("components/driver/units/router/api/Cargo.toml"));
        assert!(router_api.iter().all(|name| ["env","wire","mold","system-api"].contains(&name.as_str())));
        let router_client_manifest = fs::read_to_string(repo.join("components/driver/units/router/client/Cargo.toml")).unwrap();
        let router_client = dependencies(&repo.join("components/driver/units/router/client/Cargo.toml"));
        let router_client_api = router_client_manifest.lines().find(|line| line.trim_start().starts_with("router-api")).unwrap();
        assert!(router_client_api.contains("https://github.com/shuyra-arche/router.git"));
        assert!(router_client_api.contains("rev = \"94fec223f3c5df6c99a49527a65d016d88de3af4\""));
        assert!(!router_client_api.contains("path ="), "router-client must use canonical git router-api, not a local path");
        assert!(router_client.iter().all(|name| ["env","wire","resource","ipc","router-api"].contains(&name.as_str())));
        let router_server_manifest = fs::read_to_string(repo.join("components/driver/units/router/server/Cargo.toml")).unwrap();
        let router_server = dependencies(&repo.join("components/driver/units/router/server/Cargo.toml"));
        assert!(router_server.iter().all(|name| ["router-api","driver-shared","env","wire","resource","ipc","hub-api","system-client","system-support","unit","execution","mold","fdt"].contains(&name.as_str())));
        assert!(!router_server.iter().any(|name| ["programs","composition","router-server","system-server"].contains(&name.as_str())), "router-server depends back on Source/System implementation: {:?}", router_server);
        for name in &router_server {
            let line = router_server_manifest.lines().find(|line| line.trim_start().starts_with(&format!("{name} "))).unwrap();
            assert!(!line.contains("path ="), "router-server dependency {name} uses a local path");
        }
        for (name, url, rev) in [
            ("router-api", "https://github.com/shuyra-arche/router.git", "94fec223f3c5df6c99a49527a65d016d88de3af4"),
            ("driver-shared", "https://github.com/shuyra-arche/driver.git", "e75b1e232f828666956fb7e02c61a7d84f13eb66"),
        ] {
            let line = router_server_manifest.lines().find(|line| line.trim_start().starts_with(name)).unwrap();
            assert!(line.contains(url), "router-server {name} lacks canonical URL");
            assert!(line.contains(rev), "router-server {name} lacks selected revision");
        }
        assert!(router_server.iter().any(|name| name == "driver-shared"));
        let driver_shared_manifest = fs::read_to_string(repo.join("components/driver/shared/Cargo.toml")).unwrap();
        let driver_shared = dependencies(&repo.join("components/driver/shared/Cargo.toml"));
        assert!(driver_shared.iter().all(|name| ["env","ipc","system-api","system-client","system-support","execution","resource","hub-api","hub-client","router-api","router-client"].contains(&name.as_str())));
        assert!(!driver_shared.iter().any(|name| ["programs","composition","router-server","system-server"].contains(&name.as_str())), "driver-shared depends back on Source/System implementation: {:?}", driver_shared);
        for name in &driver_shared {
            let line = driver_shared_manifest.lines().find(|line| line.trim_start().starts_with(&format!("{name} "))).unwrap();
            assert!(!line.contains("path ="), "driver-shared dependency {name} uses a local path");
        }
        for (name, url, rev) in [
            ("router-api", "https://github.com/shuyra-arche/router.git", "94fec223f3c5df6c99a49527a65d016d88de3af4"),
            ("router-client", "https://github.com/shuyra-arche/router.git", "aa3033a7bd25b5d4199a50a6c00538475b62ef4c"),
            ("hub-client", "https://github.com/shuyra-arche/hub.git", "daa23b2611fee0c503bc6bd31a16121d97698ad6"),
        ] {
            let line = driver_shared_manifest.lines().find(|line| line.trim_start().starts_with(name)).unwrap();
            assert!(line.contains(url), "driver-shared {name} lacks canonical URL");
            assert!(line.contains(rev), "driver-shared {name} lacks selected revision");
        }
        assert!(!driver_shared.iter().any(|name| name == "router-server"));
        let composition = fs::read_to_string(repo.join("crates/composition/Cargo.toml")).unwrap();
        let driver_cluster = composition.lines().find(|line| line.trim_start().starts_with("driver-cluster")).unwrap();
        assert!(driver_cluster.contains("https://github.com/shuyra-arche/driver.git"));
        assert!(driver_cluster.contains("rev = \"b82a20f36684c13808d89b0f1a5b4ce2796c2dbf\""));
        assert!(!driver_cluster.contains("path ="), "composition driver-cluster must use selected Git, not a local path");
        let cluster = fs::read_to_string(repo.join("components/driver/cluster/Cargo.toml")).unwrap();
        for (name, url, rev) in [
            ("hub-declaration", "https://github.com/shuyra-arche/hub.git", "daa23b2611fee0c503bc6bd31a16121d97698ad6"),
            ("router-declaration", "https://github.com/shuyra-arche/router.git", "8a9416277539aefbc759c4f6519e1e8d93d63be3"),
        ] {
            let line = cluster.lines().find(|line| line.trim_start().starts_with(name)).unwrap();
            assert!(line.contains(url), "Driver cluster {name} lacks canonical URL");
            assert!(line.contains(rev), "Driver cluster {name} lacks selected revision");
            assert!(!line.contains("path ="), "Driver cluster {name} uses a local path");
        }
        let hub_client = dependencies(&repo.join("components/driver/units/hub/client/Cargo.toml"));
        assert!(hub_client.iter().all(|name| ["env","wire","resource","ipc","hub-api","system-api","system-client"].contains(&name.as_str())));
        for dependencies in [&router_api, &router_client, &hub_client] { assert!(!dependencies.iter().any(|name| name == "protocol")); }
        for source in [repo.join("components/driver/units/router/api/src"),repo.join("components/driver/units/router/client/src")] {
            let mut paths = References::default(); references(&source, &mut paths);
            assert!(!paths.0.iter().any(|path| path.starts_with("protocol::")));
        }
    }
    #[test]
    fn account_has_its_own_task_and_api_and_uses_authorized_control_contract() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let modules = fs::read_to_string(repo.join(".gitmodules")).unwrap();
        assert!(modules.contains("path = components/session"));
        assert!(modules.contains("url = https://github.com/shuyra-arche/session.git"));
        let session = repo.join("components/session");
        assert!(session.join(".git").exists(), "Session is a selected public gitlink");
        assert!(session.join("Cargo.toml").is_file());
        let service = fs::read_to_string(session.join("units/account/server/src/lib.rs")).unwrap();
        assert!(!service.contains("crate::system::"));
        assert!(service.contains("api::Call"));
        let declaration = fs::read_to_string(session.join("units/account/declaration/src/lib.rs")).unwrap();
        for required in ["pub static UNIT","manifest: \"server/Cargo.toml\"","package: \"account-server\"","bin: \"account\""] {
            assert!(declaration.contains(required), "selected Account build missing {required}");
        }
        let cluster = fs::read_to_string(session.join("cluster/src/lib.rs")).unwrap();
        assert!(cluster.contains("pub use account_declaration as account"));
        assert!(cluster.contains("pub use terminal_declaration as terminal"));
        assert!(cluster.contains("&account::UNIT"));
        assert!(cluster.contains("account::BUILD"));
        assert!(cluster.contains("manifest: \"units/account/server/Cargo.toml\""));
        assert!(cluster.contains("&terminal::UNIT"));
        assert!(cluster.contains("terminal::BUILD"));
        assert!(cluster.contains("manifest: \"units/terminal/server/Cargo.toml\""));
        let selected_account = "7dc701facdbe6368fbc09f7429920ad49dacdfca";
        for consumer in ["programs/Cargo.toml","programs/tests/control-api/Cargo.toml","programs/tests/control-rpc/Cargo.toml","programs/tests/interface-marks/Cargo.toml","programs/tests/login/Cargo.toml"] {
            let pins = fs::read_to_string(repo.join(consumer)).unwrap();
            assert!(pins.contains("https://github.com/shuyra-arche/account.git"), "{consumer} lacks the Account leaf pin");
            assert!(pins.contains(selected_account), "{consumer} lacks the selected Account gitlink");
        }
        let aggregate = fs::read_to_string(repo.join("crates/composition/Cargo.toml")).unwrap();
        assert!(aggregate.contains("https://github.com/shuyra-arche/session.git"));
        assert!(aggregate.contains("f7c7542acf0bf010c1aac07d36d30a19e0591731"), "composition lacks the selected Session aggregate");
        assert!(!aggregate.contains("terminal-declaration"));
        assert!(!repo.join("programs/src/service/account/mod.rs").exists());
        assert!(!repo.join("programs/src/service/account/api/Cargo.toml").exists());
        assert!(!repo.join("programs/src/service/account/client/Cargo.toml").exists());
        assert!(!fs::read_to_string(repo.join("programs/src/service/mod.rs")).unwrap().contains("pub mod account;"));
    }
    #[test]
    fn daemon_is_one_rtc_only_resident_supervision_process() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let declaration = fs::read_to_string(repo.join("crates/composition/src/units/daemon.rs")).unwrap();
        for required in ["name: \"daemon\"","supervision: Some(\"supervised\")","Dependency::Target(\"supervised\")","Ending::Resident","Ready::Signal(crate::unit::READY_MARK)"] { assert!(declaration.contains(required)); }
        let catalog = fs::read_to_string(repo.join("crates/composition/src/units/mod.rs")).unwrap();
        assert_eq!(catalog.matches("daemon::PROGRAM").count(), 1);
        let packages = fs::read_to_string(repo.join("crates/composition/src/image_builds.rs")).unwrap();
        assert_eq!(packages.matches("image!(\"daemon\")").count(), 1);
        assert!(fs::read_to_string(repo.join("programs/Cargo.toml")).unwrap().contains("name = \"prog-daemon\""));
        let root = repo.join("programs/src/service/daemon");
        let mut runtime = String::new();
        for entry in fs::read_dir(&root).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "rs") { runtime.push_str(&fs::read_to_string(&path).unwrap()); }
            if path.is_dir() {
                for entry in fs::read_dir(path).unwrap() {
                    let path = entry.unwrap().path();
                    if path.extension().is_some_and(|e| e == "rs") { runtime.push_str(&fs::read_to_string(path).unwrap()); }
                }
            }
        }
        for forbidden in ["Grant::State","Grant::Mint","Grant::Embark","Grant::Debark","Grant::Ruin","join_task","slay_team","oust","set_run_requested","Settle","closing","hub::","pci::","iommu::","uart::","rtc::"] { assert!(!runtime.contains(forbidden), "daemon owns {forbidden}"); }
        for required in ["control::SUPERVISION","control::RECOVER","establish::opened_by","establish::marked_as","READY_MARK"] { assert!(runtime.contains(required), "daemon missing {required}"); }
    }
}
