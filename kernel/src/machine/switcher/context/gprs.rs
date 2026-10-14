#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct Gprs(pub [usize; 32]);

#[allow(unused)]
impl Gprs {
    pub const RA: usize = 1;
    pub const SP: usize = 2;
    pub const GP: usize = 3;
    pub const TP: usize = 4;
    pub const S0: usize = 8;
    pub const A0: usize = 10;
    pub const A1: usize = 11;
    pub const A2: usize = 12;
    pub const A3: usize = 13;
    pub const A4: usize = 14;
    pub const A5: usize = 15;
    pub const A6: usize = 16;
    pub const A7: usize = 17;

    #[inline]
    pub fn x(&self, i: usize) -> usize {
        self.0[i]
    }

    #[inline]
    pub fn set_x(&mut self, i: usize, v: usize) {
        debug_assert!(i != 0, "x0 恒 0，不可写");
        self.0[i] = v;
    }
}

impl ::core::fmt::Debug for Gprs {
    fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
        write!(f, "gpr{{")?;
        let mut any = false;
        for (i, v) in self.0.iter().enumerate().skip(1) {
            if *v != 0 {
                if any {
                    write!(f, " ")?;
                }
                write!(f, "x{i}={v:#x}")?;
                any = true;
            }
        }
        write!(f, "}}")
    }
}
