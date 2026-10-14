use crate::machine::hart::HartId;
use crate::machine::layout::MAX_HART_SLOTS;

use super::MachineError;

#[derive(Clone, Copy, Debug)]
pub struct HartInfo {
    pub count: usize,
    pub hertz: usize,
}

impl HartInfo {
    pub(super) fn parse(fdt: &fdt::Fdt, boot: HartId) -> Result<Self, MachineError> {
        let boot = boot.get();
        let cpus = fdt.find_node("/cpus").ok_or(MachineError::MissingCpus)?;
        let count = cpus.children().filter(is_cpu).count();
        if count == 0 || count > MAX_HART_SLOTS || boot >= count {
            return Err(MachineError::Harts { count, boot });
        }
        let mut seen = [false; MAX_HART_SLOTS];
        for node in cpus.children().filter(is_cpu) {
            let id = node.property("reg").and_then(|property| property.as_usize());
            let Some(id) = id.filter(|id| *id < count) else {
                return Err(MachineError::Harts { count, boot });
            };
            if ::core::mem::replace(&mut seen[id], true) {
                return Err(MachineError::Harts { count, boot });
            }
        }
        Ok(Self {
            count,
            hertz: hertz(&cpus),
        })
    }
}

fn is_cpu(node: &fdt::node::FdtNode<'_, '_>) -> bool {
    node.name.split('@').next() == Some("cpu")
}

fn hertz(cpus: &fdt::node::FdtNode<'_, '_>) -> usize {
    cpus.property("timebase-frequency")
        .map(|p| match p.value.len() {
            4 => u32::from_be_bytes([p.value[0], p.value[1], p.value[2], p.value[3]]) as usize,
            8 => {
                let b = p.value;
                u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]) as usize
            }
            _ => 0,
        })
        .unwrap_or(0)
}
