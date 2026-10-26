//! 映射机制对内核的静态缝：与能力机制分开，映射夹具只实现这两格。

use env::PieToken;

pub trait MappingKernel {
    type Error;

    fn open(token: PieToken) -> Result<(usize, usize), Self::Error>;
    fn shut(token: PieToken) -> Result<(), Self::Error>;
}
