#![no_std]
extern crate alloc;
use alloc::{string::String, vec::Vec};
use env::PieToken;
use pipe_api::{Direction, Endpoint};
pub const MARK: env::Mark = env::Mark::of("shell-launch");
pub const CATALOGUE: env::Mark = env::Mark::of("shell-catalogue");
pub const MAGIC: u64 = 0x7371776172656c73;
pub const VERSION: u64 = 2;
pub const MAX_SIZE: usize = 64 * 1024;
pub const MAX_ARGS: usize = 64;
pub const MAX_PORTS: usize = 64;
pub struct Binding {
    pub name: String,
    pub endpoint: Endpoint,
}
pub struct Launch {
    pub args: Vec<String>,
    pub ports: Vec<Binding>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fail {
    Invalid,
    Full,
}
struct Decoder<'a> {
    bytes: &'a [u8],
    at: usize,
}
impl<'a> Decoder<'a> {
    fn word(&mut self) -> Option<u64> {
        let end = self.at.checked_add(8)?;
        let word = u64::from_le_bytes(self.bytes.get(self.at..end)?.try_into().ok()?);
        self.at = end;
        Some(word)
    }
    fn pie(&mut self) -> Option<PieToken> {
        let word = self.word()?;
        PieToken::from_bytes(&word.to_le_bytes())
    }
    fn string(&mut self) -> Option<String> {
        let n = usize::try_from(self.word()?).ok()?;
        let end = self.at.checked_add(n)?;
        let text = core::str::from_utf8(self.bytes.get(self.at..end)?).ok()?;
        self.at = end;
        Some(String::from(text))
    }
}
fn word(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn string(bytes: &mut Vec<u8>, text: &str) {
    word(bytes, text.len() as u64);
    bytes.extend_from_slice(text.as_bytes());
}
impl Launch {
    pub fn encode(&self) -> Result<Vec<u8>, Fail> {
        if self.args.len() > MAX_ARGS || self.ports.len() > MAX_PORTS {
            return Err(Fail::Full);
        }
        let mut bytes = Vec::new();
        word(&mut bytes, MAGIC);
        word(&mut bytes, VERSION);
        word(&mut bytes, self.args.len() as u64);
        word(&mut bytes, self.ports.len() as u64);
        for arg in &self.args {
            string(&mut bytes, arg);
        }
        for (index, binding) in self.ports.iter().enumerate() {
            if binding.name.is_empty() || self.ports[..index].iter().any(|p| p.name == binding.name)
            {
                return Err(Fail::Invalid);
            }
            string(&mut bytes, &binding.name);
            word(&mut bytes, binding.endpoint.id);
            word(&mut bytes, binding.endpoint.capacity as u64);
            word(&mut bytes, binding.endpoint.seed.get() as u64);
            word(
                &mut bytes,
                pipe_api::code(binding.endpoint.direction) as u64,
            );
        }
        if bytes.len() > MAX_SIZE {
            return Err(Fail::Full);
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Fail> {
        if bytes.len() > MAX_SIZE {
            return Err(Fail::Full);
        }
        let mut decoder = Decoder { bytes, at: 0 };
        if decoder.word() != Some(MAGIC) || decoder.word() != Some(VERSION) {
            return Err(Fail::Invalid);
        }
        let args = decoder.word().ok_or(Fail::Invalid)? as usize;
        let ports = decoder.word().ok_or(Fail::Invalid)? as usize;
        if args > MAX_ARGS || ports > MAX_PORTS {
            return Err(Fail::Full);
        }
        let mut launch = Self {
            args: Vec::new(),
            ports: Vec::new(),
        };
        launch.args.try_reserve(args).map_err(|_| Fail::Full)?;
        launch.ports.try_reserve(ports).map_err(|_| Fail::Full)?;
        for _ in 0..args {
            launch.args.push(decoder.string().ok_or(Fail::Invalid)?);
        }
        for _ in 0..ports {
            let name = decoder.string().ok_or(Fail::Invalid)?;
            if name.is_empty() || launch.ports.iter().any(|p| p.name == name) {
                return Err(Fail::Invalid);
            }
            let id = decoder.word().ok_or(Fail::Invalid)?;
            let capacity = decoder.word().ok_or(Fail::Invalid)? as usize;
            let seed = decoder.pie().ok_or(Fail::Invalid)?;
            let direction = pipe_api::direction(
                u8::try_from(decoder.word().ok_or(Fail::Invalid)?).map_err(|_| Fail::Invalid)?,
            )
            .ok_or(Fail::Invalid)?;
            if id == 0
                || capacity == 0
                || capacity > pipe_api::MAX_CAPACITY
                || seed == PieToken::NONE
            {
                return Err(Fail::Invalid);
            }
            launch.ports.push(Binding {
                name,
                endpoint: Endpoint {
                    id,
                    capacity,
                    seed,
                    direction,
                },
            });
        }
        if decoder.at != bytes.len() {
            return Err(Fail::Invalid);
        }
        Ok(launch)
    }
}
pub struct Image {
    pub name: String,
    pub seed: PieToken,
    pub length: usize,
    pub ports: Vec<(String, Direction)>,
}
pub struct Catalogue {
    pub images: Vec<Image>,
}
impl Catalogue {
    pub fn encode(&self) -> Result<Vec<u8>, Fail> {
        if self.images.len() > 32 {
            return Err(Fail::Full);
        }
        let mut bytes = Vec::new();
        word(&mut bytes, MAGIC);
        word(&mut bytes, VERSION);
        word(&mut bytes, self.images.len() as u64);
        for image in &self.images {
            string(&mut bytes, &image.name);
            word(&mut bytes, image.seed.get() as u64);
            word(&mut bytes, image.length as u64);
            word(&mut bytes, image.ports.len() as u64);
            for (name, direction) in &image.ports {
                string(&mut bytes, name);
                word(&mut bytes, pipe_api::code(*direction) as u64);
            }
        }
        if bytes.len() > MAX_SIZE {
            return Err(Fail::Full);
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, Fail> {
        if bytes.len() > MAX_SIZE {
            return Err(Fail::Full);
        }
        let mut decoder = Decoder { bytes, at: 0 };
        if decoder.word() != Some(MAGIC) || decoder.word() != Some(VERSION) {
            return Err(Fail::Invalid);
        }
        let count = decoder.word().ok_or(Fail::Invalid)? as usize;
        if count > 32 {
            return Err(Fail::Full);
        }
        let mut images: Vec<Image> = Vec::new();
        for _ in 0..count {
            let name = decoder.string().ok_or(Fail::Invalid)?;
            if name.is_empty() || images.iter().any(|image| image.name == name) {
                return Err(Fail::Invalid);
            }
            let seed = decoder.pie().ok_or(Fail::Invalid)?;
            let length = decoder.word().ok_or(Fail::Invalid)? as usize;
            let count = decoder.word().ok_or(Fail::Invalid)? as usize;
            if count > MAX_PORTS || length == 0 || seed == PieToken::NONE {
                return Err(Fail::Invalid);
            }
            let mut ports = Vec::new();
            for _ in 0..count {
                let name = decoder.string().ok_or(Fail::Invalid)?;
                if name.is_empty() || ports.iter().any(|(old, _)| old == &name) {
                    return Err(Fail::Invalid);
                }
                let direction = pipe_api::direction(
                    u8::try_from(decoder.word().ok_or(Fail::Invalid)?)
                        .map_err(|_| Fail::Invalid)?,
                )
                .ok_or(Fail::Invalid)?;
                ports.push((name, direction));
            }
            images.push(Image {
                name,
                seed,
                length,
                ports,
            });
        }
        if decoder.at != bytes.len() {
            return Err(Fail::Invalid);
        }
        Ok(Self { images })
    }
}
