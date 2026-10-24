use std::collections::VecDeque;
use hub_api::{DmaSaid, coordination::Wire};
use system_api::control::Fail;

#[derive(Default)]
pub(super) struct State {
    pub began: Vec<Wire>,
    pub replies: VecDeque<(u64, Result<Option<DmaSaid>, Fail>)>,
}
