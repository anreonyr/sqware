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
        let mut paths = References::default();
        references(&repo.join("crates/system-client/src"), &mut paths);
        assert!(!paths.0.iter().any(|path| path.starts_with("protocol::")));
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
    fn execution_mechanism_is_outside_protocol_and_common_marks_have_no_domain_dependencies() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        assert!(!repo.join("crates/protocol/src/common/schedule").exists());
        let mut paths = References::default();
        references(&repo.join("crates/schedule/src"), &mut paths);
        assert!(!paths.0.iter().any(|path| path.starts_with("protocol::")));
        let source = fs::read_to_string(repo.join("crates/protocol/src/common/marks.rs")).unwrap();
        let mut paths = References::default();
        paths.visit_file(&syn::parse_file(&source).unwrap());
        assert!(!paths.0.iter().any(|path| path.starts_with("crate::system") || path.starts_with("crate::driver") || path.starts_with("crate::service")));
    }
    #[test]
    fn mark_declarations_cannot_bypass_the_registry() {
        fn inspect(root: &Path, file: &Path, registry: &str) {
            if file.is_dir() {
                if file.file_name().is_some_and(|name| name == "tests" || name == "target") { return; }
                for entry in fs::read_dir(file).unwrap() { inspect(root, &entry.unwrap().path(), registry); }
            } else if file.extension().is_some_and(|extension| extension == "rs") {
                let source = fs::read_to_string(file).unwrap();
                let syntax = syn::parse_file(&source).unwrap();
                for item in syntax.items {
                    match item {
                        syn::Item::Macro(item) if item.mac.path.segments.last().is_some_and(|name| name.ident == "marks" || name.ident == "table") => {
                            let domain = file.parent().unwrap().strip_prefix(root).unwrap()
                                .components().map(|part| part.as_os_str().to_str().unwrap()).collect::<Vec<_>>().join("::");
                            let collection = if item.mac.path.segments.last().unwrap().ident == "marks" {
                                "marks::DECLARATIONS"
                            } else { "Grant::DECLARATIONS" };
                            assert!(registry.contains(&format!("crate::{domain}::{collection}")),
                                "{} is absent from the global mark registry", file.display());
                        }
                        syn::Item::Const(item) => if let syn::Type::Path(ty) = &*item.ty {
                            assert!(!ty.path.segments.last().is_some_and(|name| name.ident == "Mark"),
                                "{} declares {} outside marks!", file.display(), item.ident);
                        },
                        _ => {}
                    }
                }
            }
        }
        let protocol = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../crates/protocol/src");
        let registry = fs::read_to_string(protocol.join("marks.rs")).unwrap();
        inspect(&protocol, &protocol, &registry);
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
