pub struct Activity {
    pub quiet: u64,
    pub walking: bool,
}

pub fn activity() -> Result<schedule::Progress, super::Fault> {
    Ok(schedule::Progress::Done)
}
