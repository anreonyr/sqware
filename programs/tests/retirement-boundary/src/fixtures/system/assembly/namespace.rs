pub struct NamespaceInstallations;

impl NamespaceInstallations {
    pub fn program_grant(
        &self,
        _: &str,
        _: usize,
    ) -> Option<crate::system_client::operator::GrantInfo> {
        crate::backend::grant().then_some(crate::system_client::operator::GrantInfo {
            entry: crate::system_api::operator::Entry,
        })
    }
}
