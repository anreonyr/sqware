#[path = "../../../../src/system/operator/authority.rs"]
pub mod authority;

#[path = "../../../../src/system/operator/tree/mod.rs"]
pub mod tree;

pub fn associate_parts(
    operator: &mut tree::Operator,
    mark: env::Mark,
    name: &str,
    entry: system_api::operator::Entry,
) -> Result<(), system_api::operator::Fail> {
    operator.associate_namespace_parts(tree::NamespaceAssociationInput {
        mark,
        name: name.into(),
        entry,
    })
}
