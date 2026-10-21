use crate::system::app::Fault;
use crate::system::control::unit::Control;
use schedule::{Progress, Res, ResMut};

pub struct Activity {
    pub quiet: u64,
    pub walking: bool,
}

pub fn activity(control: Res<Control>, mut activity: ResMut<Activity>) -> Result<Progress, Fault> {
    activity.quiet = activity.quiet.max(control.reclaimed_at());
    activity.walking = control.walking();
    Ok(Progress::Done)
}
