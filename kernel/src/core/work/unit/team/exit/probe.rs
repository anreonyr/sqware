pub(super) fn reach(reason: usize) {
    crate::putln!("[critical] guarded exit reason={reason:#x}; no cleanup follows");
}

pub(super) fn trip(task: &crate::core::work::unit::task::Task, team: env::TeamId) {
    crate::putln!(
        "[critical] trip task={} task_team={} team={}",
        task.ident.id.get(),
        task.ident.team.id.get(),
        team.get(),
    );
}
