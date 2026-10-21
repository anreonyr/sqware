use super::{EnrollmentStep, Entry};

/// Acquired registration resources remain owned until publication is confirmed.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Enrollment {
    pub entry: Entry,
    pub step: EnrollmentStep,
}
