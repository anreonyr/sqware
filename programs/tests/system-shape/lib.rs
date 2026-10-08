#[cfg(test)]
mod tests {
    use std::{fs, path::Path};
    use syn::{
        ItemStruct, Signature,
        visit::{self, Visit},
    };
    #[derive(Default)]
    struct Signatures {
        violations: Vec<String>,
        count: usize,
    }
    impl Signatures {
        fn check(&mut self, signature: &Signature) {
            self.count += 1;
            if signature.inputs.len() > 3 {
                self.violations.push(format!(
                    "{} has {} parameters",
                    signature.ident,
                    signature.inputs.len()
                ));
            }
        }
    }
    impl<'ast> Visit<'ast> for Signatures {
        fn visit_item_struct(&mut self, item: &'ast ItemStruct) {
            const INTERMEDIATE: &[&str] = &[
                "Address",
                "Source",
                "Installation",
                "Record",
                "Declaration",
                "Location",
                "Tile",
                "Placement",
                "Registration",
                "Registrations",
                "Runtimes",
                "Image",
                "Readiness",
                "Launch",
                "Wiring",
                "Connections",
                "Internal",
                "Incoming",
                "Inbox",
                "Request",
                "Outcome",
                "Approved",
                "Kind",
                "Execution",
                "Operation",
                "Tracked",
                "Active",
                "Operations",
                "Startup",
                "Flow",
                "Activity",
                "Bound",
                "Shutoff",
                "Output",
                "Mounts",
                "Faces",
                "Book",
                "Buffer",
                "Current",
                "CurrentTip",
                "Response",
                "Ready",
                "Epoch",
                "Changed",
                "Running",
                "Tips",
                "Tip",
                "Ack",
                "LateGuests",
                "Outboxes",
                "Hit",
                "Selected",
                "Settling",
                "Judgment",
                "Membership",
                "BindingRequest",
                "Selection",
                "Subscription",
            ];
            if INTERMEDIATE.contains(&item.ident.to_string().as_str()) && item.fields.len() > 3 {
                self.violations
                    .push(format!("{} has {} fields", item.ident, item.fields.len()));
            }
            visit::visit_item_struct(self, item);
        }
        fn visit_signature(&mut self, signature: &'ast Signature) {
            self.check(signature);
            visit::visit_signature(self, signature);
        }
    }
    fn inspect(path: &Path, violations: &mut Vec<String>) -> usize {
        let mut count = 0;
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                count += inspect(&path, violations);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let source = fs::read_to_string(&path).unwrap();
                let syntax = syn::parse_file(&source)
                    .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
                let mut signatures = Signatures::default();
                signatures.visit_file(&syntax);
                count += signatures.count;
                violations.extend(
                    signatures
                        .violations
                        .into_iter()
                        .map(|violation| format!("{}: {violation}", path.display())),
                );
            }
        }
        count
    }
    #[test]
    fn every_system_function_has_at_most_three_parameters() {
        let mut violations = Vec::new();
        let count = inspect(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/system"),
            &mut violations,
        );
        assert!(count > 100, "System source tree was not inspected");
        assert!(violations.is_empty(), "{}", violations.join("\n"));
        println!("checked {count} System signatures, including receivers");
    }
}

#[cfg(test)]
mod boundaries {
    use std::{fs, path::Path};
    use syn::visit::{self, Visit};
    #[test]
    fn control_state_and_operation_queue_are_private_to_control() {
        let system = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/system");
        for (path, name) in [
            ("control/serve/unit.rs", "Control"),
            ("control/lifecycle/queue.rs", "Operations"),
        ] {
            let syntax = syn::parse_file(&fs::read_to_string(system.join(path)).unwrap()).unwrap();
            let state = syntax.items.iter().find_map(|item| match item {
                syn::Item::Struct(item) if item.ident == name => Some(item),
                _ => None,
            }).unwrap();
            assert!(!state.fields.is_empty());
            for field in &state.fields {
                match &field.vis {
                    syn::Visibility::Inherited => {},
                    syn::Visibility::Restricted(vis) => assert_eq!(
                        vis.path.segments.iter().map(|part| part.ident.to_string()).collect::<Vec<_>>(),
                        ["crate", "system", "control"],
                        "{name} exposes mutable state outside Control",
                    ),
                    _ => panic!("{name} exposes public mutable state"),
                }
            }
        }
    }
    #[test]
    fn lifecycle_dispatch_does_not_own_task_state_or_compensation() {
        let control = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/system/control");
        for old in ["serve/lifecycle", "serve/driver.rs", "serve/schedule.rs"] {
            assert!(!control.join(old).exists(), "lifecycle remains under request handling: {old}");
        }
        let mut paths = References::default();
        let source = fs::read_to_string(control.join("lifecycle/dispatch.rs")).unwrap();
        paths.visit_file(&syn::parse_file(&source).unwrap());
        assert!(!paths.0.iter().any(|path| {
            path.ends_with("::Control") || path.ends_with("::State")
                || path.ends_with("::Slot") || path.starts_with("env::")
        }), "dispatch owns domain effects: {:?}", paths.0);
        #[derive(Default)]
        struct StateFields(Vec<String>);
        impl<'a> Visit<'a> for StateFields {
            fn visit_expr_field(&mut self, field: &'a syn::ExprField) {
                if let syn::Member::Named(name) = &field.member {
                    if ["table", "instances", "loader", "pending", "failure", "deadline"].contains(&name.to_string().as_str()) {
                        self.0.push(name.to_string());
                    }
                }
                visit::visit_expr_field(self, field);
            }
        }
        let mut fields = StateFields::default();
        fields.visit_file(&syn::parse_file(&source).unwrap());
        assert!(fields.0.is_empty(), "dispatch mutates domain state: {:?}", fields.0);
    }
    #[derive(Default)]
    struct LoaderDependencies(Vec<String>);
    impl<'a> Visit<'a> for LoaderDependencies {
        fn visit_path(&mut self, path: &'a syn::Path) {
            let names: Vec<_> = path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();
            if names.windows(2).any(|pair| {
                pair[0] == "system" && matches!(pair[1].as_str(), "control" | "identity" | "run")
            }) {
                self.0.push(names.join("::"));
            }
            visit::visit_path(self, path);
        }
    }
    fn inspect_loader(root: &Path, dependencies: &mut LoaderDependencies) {
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                inspect_loader(&path, dependencies);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                dependencies
                    .visit_file(&syn::parse_file(&fs::read_to_string(path).unwrap()).unwrap());
            }
        }
    }
    #[test]
    fn loader_does_not_depend_on_lifecycle_identity_or_composition() {
        let system = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/system");
        let mut dependencies = LoaderDependencies::default();
        inspect_loader(&system.join("loader"), &mut dependencies);
        assert!(dependencies.0.is_empty(), "{}", dependencies.0.join("\n"));
    }
    #[test]
    fn identity_authority_does_not_own_control_installation_state() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let identity = repo.join("programs/src/system/identity");
        assert!(!identity.join("client").exists());
        let mut paths = References::default();
        references(&identity, &mut paths);
        assert!(!paths.0.iter().any(|path| path.contains("control::identity") || path.ends_with("::Roster")));
        let source = fs::read_to_string(repo.join("programs/src/system/control/identity.rs")).unwrap();
        let syntax = syn::parse_file(&source).unwrap();
        let roster = syntax.items.iter().find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == "Roster" => Some(item),
            _ => None,
        }).expect("Control installation state");
        assert!(matches!(&roster.vis, syn::Visibility::Restricted(vis) if vis.path.is_ident("crate")));
        assert!(roster.fields.iter().all(|field| matches!(field.vis, syn::Visibility::Inherited)));
    }
    #[test]
    fn public_clients_do_not_export_wire_data_or_server_bindings() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        for domain in ["loader", "identity", "operator", "control"] {
            let path = repo.join("crates/system-client/src").join(domain).join("mod.rs");
            let syntax = syn::parse_file(&fs::read_to_string(path).unwrap()).unwrap();
            for item in syntax.items {
                match item {
                    syn::Item::Mod(item) => if matches!(item.vis, syn::Visibility::Public(_)) {
                        assert!(!["client", "rpc", "exchange", "frame", "marks", "grant"].contains(&item.ident.to_string().as_str()));
                    },
                    syn::Item::Use(item) => if matches!(item.vis, syn::Visibility::Public(_)) {
                        let mut paths = References::default();
                        paths.visit_item_use(&item);
                        assert!(paths.0.iter().all(|path| !path.starts_with("system_api::") || path == "system_api::loader::Built"));
                    },
                    _ => {},
                }
            }
        }
        for path in ["identity/serve/face.rs", "control/serve/answer.rs", "control/serve/instance.rs", "run/loading/answer.rs", "run/account.rs", "run/publication/receive.rs", "run/names.rs"] {
            let source = fs::read_to_string(repo.join("programs/src/system").join(path)).unwrap();
            assert!(!source.contains("system_client::"), "own service binding depends on client: {path}");
        }
    }
    #[test]
    fn global_composition_and_login_policy_are_outside_control() {
        let system = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/system");
        for name in [
            "account.rs",
            "resource.rs",
            "publication",
            "install.rs",
            "run.rs",
        ] {
            assert!(
                !system.join("control/serve").join(name).exists(),
                "Control still owns {name}"
            );
        }
        for name in [
            "account.rs",
            "resource.rs",
            "publication",
            "install.rs",
            "execute.rs",
            "hooks.rs",
        ] {
            assert!(
                system.join("run").join(name).exists(),
                "missing composition module {name}"
            );
        }
    }
    #[test]
    fn provider_api_and_public_clients_keep_one_way_dependencies() {
        fn dependencies(path: &Path) -> Vec<String> {
            let source = fs::read_to_string(path).unwrap();
            let mut result = Vec::new();
            let mut dependency_section = false;
            for line in source.lines().map(str::trim) {
                if line.starts_with('[') {
                    dependency_section = line.contains("dependencies");
                } else if dependency_section {
                    if let Some((name, _)) = line.split_once('=') {
                        result.push(name.trim().to_owned());
                    }
                }
            }
            result
        }
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let api = dependencies(&repo.join("programs/src/system/api/Cargo.toml"));
        assert!(api.iter().all(|name| ["env", "wire", "mold"].contains(&name.as_str())));
        let clients = dependencies(&repo.join("crates/system-client/Cargo.toml"));
        assert!(clients.iter().any(|name| name == "system-api"));
        assert!(!clients.iter().any(|name| name == "protocol" || name == "programs"));
        let terminal_api = dependencies(&repo.join("programs/src/user/terminal/api/Cargo.toml"));
        assert!(terminal_api.iter().all(|name| ["env", "wire"].contains(&name.as_str())));
        let router_api = dependencies(&repo.join("programs/src/driver/router/api/Cargo.toml"));
        assert!(router_api.iter().all(|name| ["env", "wire", "system-api"].contains(&name.as_str())));
        let terminal_client = dependencies(&repo.join("programs/src/user/terminal/client/Cargo.toml"));
        assert!(terminal_client.iter().all(|name| ["env", "wire", "resource", "ipc", "system-client", "system-api", "terminal-api"].contains(&name.as_str())));
        assert!(terminal_client.contains(&"system-client".to_owned()));
        let router_client = dependencies(&repo.join("programs/src/driver/router/client/Cargo.toml"));
        assert!(router_client.iter().all(|name| ["env", "wire", "resource", "ipc", "router-api"].contains(&name.as_str())));
        let hub_api = dependencies(&repo.join("programs/src/service/hub/api/Cargo.toml"));
        assert!(hub_api.iter().all(|name| ["env", "wire", "mold", "system-api"].contains(&name.as_str())));
        let hub_client = dependencies(&repo.join("programs/src/service/hub/client/Cargo.toml"));
        assert!(hub_client.iter().all(|name| ["env", "wire", "resource", "ipc", "hub-api"].contains(&name.as_str())));
        for dependencies in [&terminal_api, &router_api, &hub_api, &terminal_client, &router_client, &hub_client] {
            assert!(!dependencies.iter().any(|name| name == "protocol"));
        }
        let mut paths = References::default();
        references(&repo.join("crates/system-client/src"), &mut paths);
        assert!(!paths.0.iter().any(|path| path.starts_with("protocol::")));
        for source in [
            repo.join("programs/src/user/terminal/api/src"),
            repo.join("programs/src/user/terminal/client/src"),
            repo.join("programs/src/driver/router/api/src"),
            repo.join("programs/src/driver/router/client/src"),
        ] {
            let mut paths = References::default();
            references(&source, &mut paths);
            assert!(!paths.0.iter().any(|path| path.starts_with("protocol::")));
        }
    }
    #[derive(Default)]
    struct References(Vec<String>);
    impl<'a> Visit<'a> for References {
        fn visit_path(&mut self, path: &'a syn::Path) {
            self.0.push(path.segments.iter().map(|part| part.ident.to_string()).collect::<Vec<_>>().join("::"));
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
            else if path.extension().is_some_and(|extension| extension == "rs") {
                out.visit_file(&syn::parse_file(&fs::read_to_string(path).unwrap()).unwrap());
            }
        }
    }
    #[test]
    fn request_servers_do_not_depend_on_supervisor_or_client_implementations() {
        let system = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/system");
        for domain in ["identity", "operator", "control"] {
            let mut paths = References::default();
            references(&system.join(domain).join("serve"), &mut paths);
            for path in paths.0 {
                assert!(!path.starts_with("crate::system::run") && !path.starts_with("crate::system::boot"),
                    "{domain} server depends on supervisor: {path}");
                if domain != "control" {
                    assert!(!path.starts_with("crate::system::control"), "{domain} server depends on Control: {path}");
                    assert!(!path.starts_with(&format!("crate::system::{domain}::client")),
                        "{domain} server depends on its client: {path}");
                }
            }
        }
    }
    #[test]
    fn execution_mechanism_and_environment_marks_have_no_domain_dependencies() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let mut paths = References::default();
        references(&repo.join("crates/schedule/src"), &mut paths);
        assert!(!paths.0.iter().any(|path| path.starts_with("protocol::")));
        let schedule_manifest = fs::read_to_string(repo.join("crates/schedule/Cargo.toml")).unwrap();
        assert!(!schedule_manifest.contains("protocol"));
        let source = fs::read_to_string(repo.join("crates/env/src/marks.rs")).unwrap();
        let mut paths = References::default();
        paths.visit_file(&syn::parse_file(&source).unwrap());
        assert!(!paths.0.iter().any(|path| {
            path.starts_with("crate::system")
                || path.starts_with("crate::driver")
                || path.starts_with("crate::service")
                || path.starts_with("protocol::")
        }));
    }
    #[test]
    fn mark_declarations_cannot_bypass_the_registry() {
        fn expression<'a>(syntax: &'a syn::File, name: &str) -> &'a syn::Expr {
            syntax.items.iter().find_map(|item| match item {
                syn::Item::Const(item) if item.ident == name => Some(&*item.expr),
                _ => None,
            }).unwrap_or_else(|| panic!("missing const {name}"))
        }
        #[derive(Default)]
        struct Paths(Vec<String>);
        impl<'ast> Visit<'ast> for Paths {
            fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
                self.0.push(path.path.segments.iter().map(|part| part.ident.to_string())
                    .collect::<Vec<_>>().join("::"));
                visit::visit_expr_path(self, path);
            }
        }
        #[derive(Default)]
        struct TypePaths(Vec<String>);
        impl<'ast> Visit<'ast> for TypePaths {
            fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
                self.0.push(path.path.segments.iter().map(|part| part.ident.to_string())
                    .collect::<Vec<_>>().join("::"));
                visit::visit_type_path(self, path);
            }
        }

        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let assembly = syn::parse_file(&fs::read_to_string(repo.join("programs/src/unit/interfaces.rs")).unwrap()).unwrap();
        let mut registered = Paths::default();
        registered.visit_expr(expression(&assembly, "APIS"));
        for provider in [
            "LOADER",
            "system_api::identity::REGISTRY",
            "system_api::operator::REGISTRY",
            "system_api::control::REGISTRY",
            "hub_api::REGISTRY",
            "terminal_api::REGISTRY",
            "router_api::REGISTRY",
        ] {
            assert!(registered.0.iter().any(|path| path == provider),
                "unit interface registry omits provider {provider}");
        }
        let mut loader = Paths::default();
        loader.visit_expr(expression(&assembly, "LOADER"));
        assert!(loader.0.iter().any(|path| path == "system_api::loader::REGISTRY"),
            "Loader provider registry is not assembled");

        for (api, mark_module) in [
            ("programs/src/user/terminal/api/src/lib.rs", "terminal"),
            ("programs/src/driver/router/api/src/lib.rs", "router"),
            ("programs/src/service/hub/api/src/lib.rs", "hub"),
        ] {
            let source = fs::read_to_string(repo.join(api)).unwrap();
            let syntax = syn::parse_file(&source).unwrap();
            let mut registry = Paths::default();
            registry.visit_expr(expression(&syntax, "REGISTRY"));
            assert!(registry.0.iter().any(|path| path == "marks::DECLARATIONS"),
                "{mark_module} provider does not add its mark declarations to REGISTRY");
            let item = syntax.items.iter().find_map(|item| match item {
                syn::Item::Const(item) if item.ident == "REGISTRY" => Some(item),
                _ => None,
            }).unwrap();
            let mut types = TypePaths::default();
            types.visit_type(&item.ty);
            assert!(types.0.iter().any(|path| path == "env::marks::Definition"),
                "{mark_module} provider registry no longer uses typed mark metadata");
        }
    }
    #[test]
    fn loader_frame_and_shutdown_keep_their_distinct_steps() {
        #[derive(Default)]
        struct Steps(Vec<String>);
        impl<'a> Visit<'a> for Steps {
            fn visit_expr_method_call(&mut self, call: &'a syn::ExprMethodCall) {
                if call.method == "system" {
                    if let Some(syn::Expr::Lit(literal)) = call.args.first() {
                        if let syn::Lit::Str(name) = &literal.lit { self.0.push(name.value()); }
                    }
                }
                visit::visit_expr_method_call(self, call);
            }
        }
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/system/run/loading/schedule.rs");
        let syntax = syn::parse_file(&fs::read_to_string(path).unwrap()).unwrap();
        let mut checked = 0;
        for item in syntax.items {
            if let syn::Item::Fn(function) = item {
                let expected: &[&str] = match function.sig.ident.to_string().as_str() {
                    "frame" => &["receive", "settle", "build"],
                    "shutdown" => &["withdraw", "close"],
                    _ => continue,
                };
                let mut steps = Steps::default(); steps.visit_block(&function.block);
                assert_eq!(steps.0, expected);
                checked += 1;
            }
        }
        assert_eq!(checked, 2);
    }
}
