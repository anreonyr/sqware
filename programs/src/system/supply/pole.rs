use alloc::vec;
use env::{Permission, PieKind, PieToken};
use system_api::supply::{self, Fail, Key};

use super::{LeaseRequest, Provision, ProvisionError, Resource};

/// A provider-owned shared Pole exposed through independent per-lease child roots.
pub struct PoleResource<P> {
    key: Key,
    root: PieToken,
    mapping: Option<(usize, usize)>,
    initialized: bool,
    description: [u8; supply::MAX_DESCRIPTION],
    description_len: u16,
    admission: P,
}

pub struct PoleSpec<'a, P> {
    pub key: Key,
    pub bytes: &'a [u8],
    pub description: &'a [u8],
    pub admission: P,
}

/// Effects required to build and explicitly reclaim a provider-owned Pole.
pub trait PoleEffects {
    fn unseal(&mut self, size: usize) -> Result<PieToken, Fail>;
    fn open(&mut self, token: PieToken) -> Result<(usize, usize), Fail>;
    fn write(&mut self, address: usize, bytes: &[u8]) -> Result<(), Fail>;
    fn shut(&mut self, token: PieToken) -> Result<(), Fail>;
    fn inspect(&mut self, token: PieToken) -> Result<env::PieInfo, Fail>;
    fn release(&mut self, token: PieToken) -> Result<(), Fail>;
}

struct NativePoleEffects;

impl PoleEffects for NativePoleEffects {
    fn unseal(&mut self, size: usize) -> Result<PieToken, Fail> {
        ::resource::raw::pole(size, true).map_err(|_| Fail::Full)
    }
    fn open(&mut self, token: PieToken) -> Result<(usize, usize), Fail> {
        resource::raw::open(token).map_err(|_| Fail::SourceLost)
    }
    fn write(&mut self, address: usize, bytes: &[u8]) -> Result<(), Fail> {
        // The open Pole mapping returned by the kernel covers this write.
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), address as *mut u8, bytes.len());
        }
        Ok(())
    }
    fn shut(&mut self, token: PieToken) -> Result<(), Fail> {
        env::pie::shut(token).map_err(|_| Fail::SourceLost)
    }
    fn inspect(&mut self, token: PieToken) -> Result<env::PieInfo, Fail> {
        resource::raw::inspect(token).map_err(|_| Fail::SourceLost)
    }
    fn release(&mut self, token: PieToken) -> Result<(), Fail> {
        resource::raw::release(token).map_err(|_| Fail::SourceLost)
    }
}

/// A failed constructor retains any Pole and mapping it already created.
#[must_use = "retry cleanup or transfer the retained resource before discarding initialization failure"]
pub struct InitError<P> {
    failure: Fail,
    cleanup_failure: Option<Fail>,
    resource: Option<PoleResource<P>>,
}

impl<P> core::fmt::Debug for InitError<P> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("InitError")
            .field("failure", &self.failure())
            .field("cleanup_failure", &self.cleanup_failure())
            .field("retained", &self.resource.is_some())
            .finish()
    }
}

impl<P> InitError<P> {
    pub fn failure(&self) -> Fail {
        self.failure
    }
    pub fn cleanup_failure(&self) -> Option<Fail> {
        self.cleanup_failure
    }

    #[allow(dead_code, reason = "Transfers a retained failed resource as an alternative to retrying cleanup.")]
    pub fn into_resource(self) -> Option<PoleResource<P>> {
        self.resource
    }
}

impl<P> InitError<P>
where
    P: Fn(&LeaseRequest) -> bool,
{
    pub fn retry_cleanup(&mut self) -> Result<(), Fail> {
        self.retry_cleanup_with(&mut NativePoleEffects)
    }

    pub fn retry_cleanup_with(&mut self, effects: &mut impl PoleEffects) -> Result<(), Fail> {
        let Some(resource) = self.resource.as_mut() else {
            self.cleanup_failure = None;
            return Ok(());
        };
        match resource.cleanup_root_with(effects) {
            Ok(()) => {
                self.resource = None;
                self.cleanup_failure = None;
                Ok(())
            }
            Err(failure) => {
                self.cleanup_failure = Some(failure);
                Err(failure)
            }
        }
    }
}

impl<P> PoleResource<P>
where
    P: Fn(&LeaseRequest) -> bool,
{
    pub fn from_bytes(spec: PoleSpec<'_, P>) -> Result<Self, InitError<P>> {
        Self::from_bytes_with(spec, &mut NativePoleEffects)
    }

    pub fn from_bytes_with(
        spec: PoleSpec<'_, P>,
        effects: &mut impl PoleEffects,
    ) -> Result<Self, InitError<P>> {
        let PoleSpec {
            key,
            bytes,
            description,
            admission,
        } = spec;
        if key.name().is_empty() || bytes.is_empty() || description.len() > supply::MAX_DESCRIPTION
        {
            return Err(InitError {
                failure: Fail::Bad,
                cleanup_failure: None,
                resource: None,
            });
        }
        let size = bytes
            .len()
            .checked_add(env::PAGE_SIZE - 1)
            .map(|size| size / env::PAGE_SIZE * env::PAGE_SIZE)
            .filter(|size| *size != 0)
            .ok_or_else(|| InitError {
                failure: Fail::Bad,
                cleanup_failure: None,
                resource: None,
            })?;
        let root = effects.unseal(size).map_err(|failure| InitError {
            failure,
            cleanup_failure: None,
            resource: None,
        })?;
        let mut descriptor = [0; supply::MAX_DESCRIPTION];
        descriptor[..description.len()].copy_from_slice(description);
        let mut resource = Self {
            key,
            root,
            mapping: None,
            initialized: false,
            description: descriptor,
            description_len: description.len() as u16,
            admission,
        };
        let initialized = resource.initialize_with(bytes, effects);
        match initialized {
            Ok(()) => Ok(resource),
            Err(failure) => {
                let mut error = InitError {
                    failure,
                    cleanup_failure: None,
                    resource: Some(resource),
                };
                let _ = error.retry_cleanup_with(effects);
                Err(error)
            }
        }
    }

    fn revoke_root(&mut self) -> Result<(), Fail> {
        self.cleanup_root_with(&mut NativePoleEffects)
    }

    fn initialize_with(
        &mut self,
        bytes: &[u8],
        effects: &mut impl PoleEffects,
    ) -> Result<(), Fail> {
        let (address, mapped) = effects.open(self.root)?;
        self.mapping = Some((address, mapped));
        if mapped < bytes.len() {
            return Err(Fail::SourceLost);
        }
        effects.write(address, bytes)?;
        effects.shut(self.root)?;
        self.mapping = None;
        self.initialized = true;
        Ok(())
    }

    fn cleanup_root_with(&mut self, effects: &mut impl PoleEffects) -> Result<(), Fail> {
        if let Some((_, _)) = self.mapping {
            effects.shut(self.root)?;
            self.mapping = None;
        }
        if self.root == PieToken::NONE {
            return Ok(());
        }
        let info = effects.inspect(self.root)?;
        if !info.alive {
            self.root = PieToken::NONE;
            return Ok(());
        }
        effects.release(self.root)?;
        self.root = PieToken::NONE;
        Ok(())
    }
}

impl<P> Resource for PoleResource<P>
where
    P: Fn(&LeaseRequest) -> bool,
{
    fn key(&self) -> Key {
        self.key
    }

    fn authorize(&self, request: &LeaseRequest) -> bool {
        if !self.initialized {
            return false;
        }
        let Ok(info) = resource::raw::inspect(self.root) else {
            return false;
        };
        let mut needed = request.permission | Permission::VEST;
        if info.permission.contains(Permission::ONLY) {
            needed |= Permission::ONLY;
        }
        info.alive
            && info.kind == PieKind::Pole
            && info.permission.contains(needed)
            && request.permission.contains(Permission::ONLY)
                == info.permission.contains(Permission::ONLY)
            && (self.admission)(request)
    }

    fn acquire(&mut self, request: &LeaseRequest) -> Result<Provision, ProvisionError> {
        if !self.authorize(request) {
            return Err(ProvisionError::new(Fail::Denied));
        }
        let root = request.derive(self.root, 0).map_err(ProvisionError::new)?;
        Provision::new(
            vec![root],
            0,
            &self.description[..self.description_len as usize],
        )
    }

    fn shutdown(&mut self) -> Result<(), Fail> {
        self.revoke_root()
    }
}
