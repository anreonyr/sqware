use env::TaskId;
use shell_api::Launch;

pub struct Manifest;

impl Manifest {
    pub fn install(_: &Launch, _: TaskId) -> Result<Self, &'static str> {
        Ok(Manifest)
    }
}
