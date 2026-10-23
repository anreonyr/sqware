pub struct Readiness<'a> {
    pub name: &'a str,
    pub marks: &'a [crate::Mark],
    pub wait: crate::Wait,
}
