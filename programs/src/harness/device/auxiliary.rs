use core::sync::atomic::{AtomicUsize, Ordering};

use env::{PieToken, TaskId, TeamId};

static SENSOR: AtomicUsize = AtomicUsize::new(0);

pub fn spawn(controller: TaskId) -> Result<(TaskId, PieToken), &'static str> {
    let task = execution::unit::spawn(
        TeamId::new(0),
        entry as *const () as usize,
        &[controller.get()],
        0,
    )
    .map_err(|_| "work: auxiliary spawn")?;
    env::unit::embark_task(task).map_err(|_| "work: auxiliary embark")?;
    let until = env::chrono::clock().saturating_add(3_000_000_000);
    loop {
        let sensor = SENSOR.load(Ordering::Acquire);
        if sensor != 0 {
            return Ok((task, PieToken::mint(sensor)));
        }
        if env::chrono::clock() >= until {
            return Err("work: auxiliary startup");
        }
        let _ = execution::room::park(core::time::Duration::from_millis(1));
    }
}

extern "C" fn entry(args: usize) -> ! {
    // Spawn places its immutable argument words in this task's own Space.
    let controller = TaskId::new(unsafe { (args as *const usize).read() });
    let sensor = env::hole::unseal(hub_api::ALIVE_MARK, env::HoleLimits::default())
        .expect("work: auxiliary sensor");
    let sensor = resource::port::ship(
        sensor,
        controller,
        env::Access::FETCH_STORE,
        env::Policy::NONE,
    )
    .expect("work: auxiliary sensor handover")
    .seed();
    SENSOR.store(sensor.get(), Ordering::Release);
    crate::debug::put(&alloc::format!(
        "work: auxiliary running tid={}",
        env::unit::self_id().get()
    ));
    loop {
        for _ in 0..1024 {
            core::hint::spin_loop();
        }
        execution::room::park(core::time::Duration::from_millis(1))
            .expect("work: auxiliary park");
    }
}
