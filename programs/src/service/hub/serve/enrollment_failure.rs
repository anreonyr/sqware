#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnrollmentFailure {
    Identity(system_client::identity::CallError),
    Operator(system_api::operator::Fail),
    Door(env::HoleFail),
    Layout,
}
