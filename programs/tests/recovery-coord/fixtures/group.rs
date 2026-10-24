use alloc::rc::Rc;
use core::cell::RefCell;

use crate::{coord::Coordinator, peers::Peers, stage::Stage};
use super::script::Script;

pub(crate) struct Group {
    pub coords: Vec<Coordinator>,
    pub peers: Peers,
    pub script: Rc<RefCell<Script>>,
    pub stages: Vec<Vec<Stage>>,
}
