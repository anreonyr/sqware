#[cfg(test)]
mod tests {
    use std::{fs, path::Path};
    use syn::{visit::{self, Visit}, Signature, ItemStruct};
    #[derive(Default)]
    struct Signatures { violations: Vec<String>, count: usize }
    impl Signatures {
        fn check(&mut self, signature: &Signature) {
            self.count += 1;
            if signature.inputs.len() > 3 {
                self.violations.push(format!("{} has {} parameters", signature.ident, signature.inputs.len()));
            }
        }
    }
    impl<'ast> Visit<'ast> for Signatures {
        fn visit_item_struct(&mut self, item: &'ast ItemStruct) {
            const INTERMEDIATE: &[&str] = &["Address", "Source", "Installation", "Record", "Declaration", "Location", "Tile", "Placement", "Registration", "Registrations", "Runtimes", "Image", "Readiness", "Launch", "Wiring", "Connections", "Internal", "Incoming", "Inbox", "Request", "Outcome", "Approved", "Kind", "Execution", "Operation", "Tracked", "Active", "Operations", "Startup", "Flow", "Activity", "Bound", "Shutoff", "Output", "Mounts", "Faces", "Book", "Buffer", "Current", "CurrentTip", "Response", "Running", "Tips", "Tip", "Ack", "LateGuests", "Outboxes", "Hit", "Selected", "Settling", "Judgment", "Membership", "BindingRequest", "Selection", "Subscription"];
            if INTERMEDIATE.contains(&item.ident.to_string().as_str()) && item.fields.len() > 3 {
                self.violations.push(format!("{} has {} fields", item.ident, item.fields.len()));
            }
            visit::visit_item_struct(self, item);
        }
        fn visit_signature(&mut self, signature: &'ast Signature) {
            self.check(signature); visit::visit_signature(self, signature);
        }
    }
    fn inspect(path: &Path, violations: &mut Vec<String>) -> usize {
        let mut count = 0;
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() { count += inspect(&path, violations); }
            else if path.extension().is_some_and(|ext| ext == "rs") {
                let source = fs::read_to_string(&path).unwrap();
                let syntax = syn::parse_file(&source).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
                let mut signatures = Signatures::default(); signatures.visit_file(&syntax);
                count += signatures.count;
                violations.extend(signatures.violations.into_iter().map(|violation| format!("{}: {violation}", path.display())));
            }
        }
        count
    }
    #[test]
    fn every_system_function_has_at_most_three_parameters() {
        let mut violations = Vec::new();
        let count = inspect(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/system"), &mut violations);
        assert!(count > 100, "System source tree was not inspected");
        assert!(violations.is_empty(), "{}", violations.join("\n"));
        println!("checked {count} System signatures, including receivers");
    }
}
