mod source;
pub mod unit;

fn fail(source: env::UnitFail) -> erra::Error<env::UnitFail> {
    erra::Error::new("constructing ELF image", source)
}
pub mod build;
