use env::Access;
use system_api::operator::Permit;

#[derive(Clone, Copy)]
pub struct Binding {
    pub name: &'static str,
    /// Locator only; composition must validate the resolved live resource before binding it.
    pub mark: env::Mark,
    pub view: Access,
    pub permit: Permit,
}
