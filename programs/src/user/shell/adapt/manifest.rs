use env::{PieToken, TaskId};
use shell_api::Launch;
pub struct Manifest {
    token: PieToken,
}
impl Manifest {
    pub fn install(launch: &Launch, task: TaskId) -> Result<Self, &'static str> {
        let bytes = launch.encode().map_err(|_| "invalid launch manifest")?;
        let size = (bytes.len() + 8).div_ceil(env::PAGE_SIZE) * env::PAGE_SIZE;
        let token = env::pie::unseal(env::UnsealArgs::Pole { size, shared: true })
            .map_err(|_| "manifest allocation")?;
        let manifest = Self { token };
        let (address, mapped) = resource::raw::open(token).map_err(|_| "manifest mapping")?;
        if mapped < bytes.len() + 8 {
            return Err("short manifest mapping");
        }
        // SAFETY: the private writable mapping covers the length and complete encoded manifest.
        unsafe {
            core::ptr::copy_nonoverlapping(
                (bytes.len() as u64).to_le_bytes().as_ptr(),
                address as *mut u8,
                8,
            );
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), (address + 8) as *mut u8, bytes.len());
        }
        env::pie::shut(token).map_err(|_| "manifest unmap")?;
        env::pie::narrow(token, env::Permission::FETCH | env::Permission::VEST)
            .map_err(|_| "manifest freeze")?;
        env::pie::accord(token, task, env::Permission::FETCH, shell_api::MARK)
            .map_err(|_| "manifest grant")?;
        Ok(manifest)
    }
}
impl Drop for Manifest {
    fn drop(&mut self) {
        let _ = env::pie::release(self.token, env::ReleaseMode::Revoke);
    }
}
