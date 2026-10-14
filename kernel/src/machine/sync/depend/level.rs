#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(u8)]
pub enum Level {
    Unit = 0,
    Scheduler = 1,
    Space = 2,
    Nest = 3,
    View = 4,
    Flat = 5,
    TeamTasks = 6,
    L3 = 7,
    Roster = 8,
    Asid = 9,
    Frame = 10,
    Block = 11,
    Tally = 12,
    Spare = 13,
    UnitState = 14,
}
