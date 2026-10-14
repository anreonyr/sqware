#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    Plain = 0,
    Trap,
    Lazy,
    Heap,
    Stack,
    Image,
    Pole,
    Table,
    TrapStack,
    HartFrame,
    Spare,
    Prime,
    Probe,
    Task,
    Team,
    Space,
}

impl Kind {
    pub(crate) const COUNT: usize = Self::ALL.len();

    pub(crate) const ALL: [Kind; 16] = [
        Kind::Plain,
        Kind::Trap,
        Kind::Lazy,
        Kind::Heap,
        Kind::Stack,
        Kind::Image,
        Kind::Pole,
        Kind::Table,
        Kind::TrapStack,
        Kind::HartFrame,
        Kind::Spare,
        Kind::Prime,
        Kind::Probe,
        Kind::Task,
        Kind::Team,
        Kind::Space,
    ];

    pub(crate) const fn ix(self) -> usize {
        self as usize
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Kind::Plain => "plain",
            Kind::Trap => "trap",
            Kind::Lazy => "lazy",
            Kind::Heap => "heap",
            Kind::Stack => "stack",
            Kind::Image => "image",
            Kind::Pole => "pole",
            Kind::Table => "table",
            Kind::TrapStack => "trap-stack",
            Kind::HartFrame => "hart-frame",
            Kind::Spare => "spare",
            Kind::Prime => "prime",
            Kind::Probe => "probe",
            Kind::Task => "task",
            Kind::Team => "team",
            Kind::Space => "space",
        }
    }
}
