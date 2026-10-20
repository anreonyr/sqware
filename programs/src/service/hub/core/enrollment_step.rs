use system_api::operator::Entry;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnrollmentStep {
    Door,
    Part,
    Bind(Entry),
}
