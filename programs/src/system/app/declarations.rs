use unit::UnitFile;

/// Borrowed declaration set shared by one System run; never a second table copy.
#[derive(Clone, Copy)]
pub(crate) struct Declarations(pub(crate) &'static [&'static UnitFile]);

impl Declarations {
    pub(crate) fn find(&self, name: &str) -> Option<&'static UnitFile> {
        self.0
            .iter()
            .copied()
            .find(|program| program.name() == name)
    }

    pub(crate) fn list(&self) -> &'static [&'static UnitFile] {
        self.0
    }
}
