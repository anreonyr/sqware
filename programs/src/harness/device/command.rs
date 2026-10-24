use env::PieToken;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Command {
    Window(PieToken),
    Ring,
    Close,
    Exit,
    Probe,
    Submit,
}

impl Command {
    pub const LEN: usize = 1 + PieToken::WIDTH;

    pub fn store(self, out: &mut [u8; Self::LEN]) -> usize {
        out[0] = match self {
            Self::Window(_) => 1,
            Self::Ring => 2,
            Self::Close => 3,
            Self::Exit => 4,
            Self::Probe => 5,
            Self::Submit => 6,
        };
        if let Self::Window(token) = self {
            out[1..].copy_from_slice(&token.to_bytes());
            Self::LEN
        } else {
            1
        }
    }

    pub fn fetch(bytes: &[u8]) -> Option<Self> {
        match bytes {
            [1, rest @ ..] if bytes.len() == Self::LEN => {
                let token = PieToken::from_bytes(rest)?;
                (token != PieToken::NONE).then_some(Self::Window(token))
            }
            [2] => Some(Self::Ring),
            [3] => Some(Self::Close),
            [4] => Some(Self::Exit),
            [5] => Some(Self::Probe),
            [6] => Some(Self::Submit),
            _ => None,
        }
    }
}
