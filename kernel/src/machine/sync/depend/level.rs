#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[repr(u8)]
pub enum Level {
    Unit = 0,
    Scheduler = 1,
    Space = 2,
    Tree = 3,
    Map = 4,
    TeamTasks = 5,
    L3 = 6,
    Roster = 7,
    Asid = 8,
    Frame = 9,
    Block = 10,
    Tally = 11,
    Spare = 12,
    UnitState = 13,
}
