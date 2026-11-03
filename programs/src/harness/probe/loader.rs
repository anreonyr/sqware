pub(crate) fn acceptance(
    assembly: &mut system_server::test_support::Fixture,
    operator: &system_client::operator::Face,
) {
    system_server::test_support::loader_scenario::acceptance(assembly, operator);
}
