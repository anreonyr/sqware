mod control_face;
mod instance;

pub use control_face::ControlFace;
pub use instance::Instance;

pub struct Peers {
    pub control: ControlFace,
}
