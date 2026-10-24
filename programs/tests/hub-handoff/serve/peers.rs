mod control_face;
mod instance;
mod coord_face;

pub use control_face::ControlFace;
pub use instance::Instance;
pub use coord_face::CoordFace;

pub struct Peers {
    pub control: ControlFace,
    pub coord: CoordFace,
}
