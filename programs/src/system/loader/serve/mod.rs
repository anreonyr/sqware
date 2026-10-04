mod source;
pub mod unit;

fn fail(source: env::UnitFail) -> erra::Error<env::UnitFail> {
    erra::Error::new("constructing ELF image", source)
}
pub mod answer;
pub mod build;
pub mod frame;
pub mod install;
pub mod schedule;

pub mod publication;

pub mod watch;
