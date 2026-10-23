use crate::system::control::unit::{
    table::{Slot, State, Table},
    verdict::Fail,
    Input, Pending,
};
use alloc::vec::Vec;

pub struct Control {
    pub table: Table,
    pub inputs: Vec<Input>,
    pub pending: Vec<Pending>,
    pub guarded: &'static [&'static str],
    pub guard_token: Option<crate::PieToken>,
}

impl Control {
    pub fn input(&self, name: &str) -> Result<Input, Fail> {
        self.inputs
            .iter()
            .copied()
            .find(|input| input.program.name() == name)
            .ok_or(Fail::Unknown)
    }

    pub fn task(&self, name: &str) -> Option<crate::TaskId> {
        match self.table.find(name)?.slot {
            Slot::Live { task, .. } => Some(task),
            Slot::None => None,
        }
    }

    pub fn task_team(&self, task: crate::TaskId) -> Option<crate::TeamId> {
        self.table.rows().find_map(|row| match row.slot {
            Slot::Live { task: live, team } if live == task => team,
            _ => None,
        })
    }

    pub fn guard_required(&self, name: &str) -> bool {
        self.guarded.contains(&name)
    }

    pub fn guard_held(&self, name: &str) -> bool {
        self.table.guard_held(name)
    }

    pub fn hold_guard(&mut self, name: &str) {
        self.table.set_guard_held(name, true);
    }

    pub fn clear_guard(&mut self, name: &str) {
        self.table.set_guard_held(name, false);
    }

    pub fn resume(&mut self, name: &str) -> Result<(), Fail> {
        let row = self.table.find(name).ok_or(Fail::Unknown)?;
        if row.state != State::Debarked {
            return Err(Fail::NotReady);
        }
        let Slot::Live {
            team: Some(team), ..
        } = row.slot
        else {
            return Err(Fail::Unknown);
        };
        crate::unit::embark_team(team).map_err(|_| Fail::NotReady)?;
        self.table.set_state(name, State::Ready);
        Ok(())
    }

    pub fn detach_unit(&mut self, name: &str) -> Result<(), Fail> {
        self.table.detach(name)?;
        self.table.set_state(name, State::Dead);
        Ok(())
    }
}
