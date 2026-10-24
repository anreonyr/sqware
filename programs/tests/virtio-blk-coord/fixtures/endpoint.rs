use env::PieToken;

pub struct Endpoint;

impl Endpoint {
    pub fn rx(&self) -> PieToken { PieToken::mint(0xc1) }
    pub fn seed(&self) -> PieToken { PieToken::mint(0xc2) }
}
