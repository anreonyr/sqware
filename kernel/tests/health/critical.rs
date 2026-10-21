use crate::core::work::room::messenger;
use crate::core::work::unit::space::SpaceBuilder;
use crate::core::work::unit::team::TeamBuilder;

pub fn run() -> ! {
    crate::core::work::room::scheduler::init().expect("scheduler");
    crate::entry::boot::register_runtime_hooks();
    crate::entry::boot::boot_harts().expect("peer harts");
    let me = crate::machine::hart::hart_id();
    for id in 0..crate::machine::hart::hart_count() {
        let id = crate::machine::hart::HartId::new(id);
        while id != me && !crate::machine::hart::online(id) {
            core::hint::spin_loop();
        }
    }
    let team = TeamBuilder::new(SpaceBuilder::user().build().expect("space"))
        .spawn()
        .expect("team");
    let task = team.task().hold().expect("task");
    team.debark().expect("debark");
    team.guard(task.ident.id).expect("guard");
    crate::putln!(
        "[critical] armed team={} task={}",
        team.id.get(),
        task.ident.id.get()
    );
    messenger::slay(&task);
    unreachable!("critical guard returned after slay")
}
