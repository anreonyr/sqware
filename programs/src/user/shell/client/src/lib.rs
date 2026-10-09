#![no_std]
extern crate alloc;
use alloc::{string::String, vec::Vec};
use pipe_client::{Direction, Port, Read, Write};
pub struct Boot {
    pub args: Vec<String>,
    ports: Vec<(String, Port)>,
}
impl Boot {
    pub fn take() -> Result<Self, ()> {
        let owner = env::TaskId::new(*execution::boot::args::args().first().ok_or(())?);
        let entry = resource::raw::pies()
            .find(|info| {
                info.alive
                    && info.owner == owner
                    && info.vestor == owner
                    && info.mark == shell_api::MARK
                    && info.kind == env::PieKind::Pole
            })
            .ok_or(())?;
        let dock = resource::dock::Dock::open(entry.token).map_err(|_| ())?;
        let view = dock.view();
        if view.size() < 8 {
            return Err(());
        }
        // SAFETY: the startup grant is read-only and bounded by its retained Dock.
        let bytes = unsafe { core::slice::from_raw_parts(view.base() as *const u8, view.size()) };
        let len = u64::from_le_bytes(bytes[..8].try_into().map_err(|_| ())?) as usize;
        let launch =
            shell_api::Launch::decode(bytes.get(8..8usize.checked_add(len).ok_or(())?).ok_or(())?)
                .map_err(|_| ())?;
        let mut ports = Vec::new();
        for binding in launch.ports {
            ports.push((
                binding.name,
                Port::import(binding.endpoint).map_err(|_| ())?,
            ));
        }
        dock.shut().map_err(|_| ())?;
        let _ = env::pie::release(entry.token, env::ReleaseMode::Revoke);
        Ok(Self {
            args: launch.args,
            ports,
        })
    }
    pub fn port(&mut self, name: &str, direction: Direction) -> Result<Port, ()> {
        let at = self
            .ports
            .iter()
            .position(|(key, port)| key == name && port.direction() == direction)
            .ok_or(())?;
        Ok(self.ports.swap_remove(at).1)
    }
}
pub fn copy(mut read: Port, mut write: Port, uppercase: bool) -> Result<(), ()> {
    let mut buffer = [0; 256];
    loop {
        match read.read(&mut buffer, env::Wait::Forever).map_err(|_| ())? {
            Read::Bytes(n) => {
                if uppercase {
                    buffer[..n].make_ascii_uppercase();
                }
                write_all(&mut write, &buffer[..n])?;
            }
            Read::Pending => {}
            Read::Eof => {
                write.close().map_err(|_| ())?;
                read.close().map_err(|_| ())?;
                return Ok(());
            }
        }
    }
}
pub fn write_all(port: &mut Port, mut bytes: &[u8]) -> Result<(), ()> {
    while !bytes.is_empty() {
        match port.write(bytes, env::Wait::Forever).map_err(|_| ())? {
            Write::Bytes(n) if n > 0 => bytes = &bytes[n..],
            Write::Pending => {}
            _ => return Err(()),
        }
    }
    Ok(())
}
