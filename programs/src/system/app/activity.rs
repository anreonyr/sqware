use crate::system::app::Fault;
use crate::system::control::unit::Control;
use schedule::{Progress, Res, ResMut};

pub struct Activity {
    pub owed: usize,
    pub quiet: u64,
    pub walking: bool,
}

pub fn activity(control: Res<Control>, mut activity: ResMut<Activity>) -> Result<Progress, Fault> {
    let living = control.living_count()
        + control
            .instances()
            .filter(|instance| instance.team.is_some())
            .count();
    if activity.owed == 0 || living < activity.owed {
        activity.quiet = env::chrono::clock();
    }
    activity.owed = living;
    activity.walking = control.walking();
    Ok(Progress::Done)
}
