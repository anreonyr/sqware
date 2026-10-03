use alloc::sync::Arc;
use core::alloc::Layout;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use env::{Permission, TeamId};

use crate::memory::PAGE_SIZE;
use crate::memory::allocator::frame;
use crate::memory::manager::{MapError, addr::PhysAddr};

#[derive(Debug)]
pub(crate) struct Backing {
    base: NonNull<u8>,
    size: usize,
    owned: bool,
    holders: AtomicUsize,
    writers: AtomicUsize,
    writable_maps: AtomicUsize,
    aliases: AtomicUsize,
    operating: AtomicBool,
    reserved: AtomicUsize,
}

// SAFETY: access to shared physical pages is controlled by capability and PTE permissions.
unsafe impl Send for Backing {}
unsafe impl Sync for Backing {}

impl Backing {
    pub(crate) fn allocate(size: usize) -> Result<Arc<Self>, MapError> {
        if size == 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        let layout = Layout::from_size_align(size, PAGE_SIZE).map_err(|_| MapError::NotAligned)?;
        let allocation = crate::tag!(Pole, frame::allocator().allocate(layout))
            .map_err(|_| MapError::OutOfMemory)?;
        let base = allocation.cast::<u8>();
        // SAFETY: the allocation covers size bytes and has not been published.
        unsafe { core::ptr::write_bytes(base.as_ptr(), 0, size) };
        Arc::try_new(Self::new(base, size, true)).map_err(|_| MapError::OutOfMemory)
    }

    pub(crate) fn region(base: usize, size: usize) -> Result<Arc<Self>, MapError> {
        if size == 0 || !base.is_multiple_of(PAGE_SIZE) || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        base.checked_add(size).ok_or(MapError::NoRegion)?;
        let base = NonNull::new(base as *mut u8).ok_or(MapError::NoRegion)?;
        Arc::try_new(Self::new(base, size, false)).map_err(|_| MapError::OutOfMemory)
    }

    fn new(base: NonNull<u8>, size: usize, owned: bool) -> Self {
        Self { base, size, owned, holders: AtomicUsize::new(0), writers: AtomicUsize::new(0),
            writable_maps: AtomicUsize::new(0), aliases: AtomicUsize::new(0), operating: AtomicBool::new(false), reserved: AtomicUsize::new(0) }
    }

    pub(crate) fn operation(self: &Arc<Self>) -> Option<Operation> {
        self.operating.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).ok()?;
        Some(Operation(self.clone()))
    }

    pub(crate) fn readonly(&self) -> bool {
        self.writers.load(Ordering::Acquire) == 0 && self.writable_maps.load(Ordering::Acquire) == 0
            && self.reserved.load(Ordering::Acquire) == 0
    }

    pub(crate) fn exclusive(&self) -> bool {
        self.holders.load(Ordering::Acquire) == 1 && self.reserved.load(Ordering::Acquire) == 0
    }

    pub(crate) fn reserved(&self) -> usize { self.reserved.load(Ordering::Acquire) }
    pub(crate) fn reserve(&self, team: TeamId) { self.reserved.store(team.get(), Ordering::Release); }
    pub(crate) fn unreserve(&self) { self.reserved.store(0, Ordering::Release); }

    pub(crate) fn unmapped(&self) -> bool { self.aliases.load(Ordering::Acquire) == 0 }
    pub(super) fn alias(&self, add: bool) {
        if add { self.aliases.fetch_add(1, Ordering::AcqRel); }
        else { self.aliases.fetch_sub(1, Ordering::AcqRel); }
    }

    pub(super) fn map_write(&self, add: bool) {
        if add { self.writable_maps.fetch_add(1, Ordering::AcqRel); }
        else { self.writable_maps.fetch_sub(1, Ordering::AcqRel); }
    }

    #[cfg(debug_assertions)]
    pub(crate) fn permit(self: &Arc<Self>, permission: Permission) -> Arc<Permit> {
        self.try_permit(permission).expect("permit: allocation")
    }

    pub(crate) fn try_permit(self: &Arc<Self>, permission: Permission) -> Result<Arc<Permit>, MapError> {
        self.holders.fetch_add(1, Ordering::AcqRel);
        if permission.contains(Permission::STORE) { self.writers.fetch_add(1, Ordering::AcqRel); }
        // An allocation failure drops Permit and reverses these registrations.
        Arc::try_new(Permit { backing: self.clone(), permission: AtomicU32::new(permission.bits()), live: AtomicBool::new(true) })
            .map_err(|_| MapError::OutOfMemory)
    }

    pub(crate) fn size(&self) -> usize {
        self.size
    }

    pub(crate) fn owned(&self) -> bool {
        self.owned
    }

    pub(crate) fn address(&self, offset: usize, size: usize) -> Result<PhysAddr, MapError> {
        if !offset.is_multiple_of(PAGE_SIZE) || size == 0 || !size.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::NotAligned);
        }
        if offset.checked_add(size).is_none_or(|end| end > self.size) {
            return Err(MapError::NoRegion);
        }
        Ok(PhysAddr::from_raw(self.base.as_ptr() as usize + offset))
    }
}

pub(crate) struct Operation(Arc<Backing>);
impl Drop for Operation {
    fn drop(&mut self) { self.0.operating.store(false, Ordering::Release); }
}

pub(crate) struct Permit {
    backing: Arc<Backing>,
    permission: AtomicU32,
    live: AtomicBool,
}

impl Permit {
    pub(crate) fn permission(&self) -> Permission {
        if !self.live.load(Ordering::Acquire) { return Permission::empty(); }
        Permission::from_bits_retain(self.permission.load(Ordering::Acquire))
    }
    pub(crate) fn narrow(&self, subset: Permission) {
        let previous = self.permission.swap(subset.bits(), Ordering::AcqRel);
        if previous & Permission::STORE.bits() != 0 && !subset.contains(Permission::STORE) {
            self.backing.writers.fetch_sub(1, Ordering::AcqRel);
        }
    }
    pub(crate) fn invalidate(&self) {
        if self.live.swap(false, Ordering::AcqRel) {
            self.backing.holders.fetch_sub(1, Ordering::AcqRel);
            if self.permission.load(Ordering::Acquire) & Permission::STORE.bits() != 0 {
                self.backing.writers.fetch_sub(1, Ordering::AcqRel);
            }
        }
    }
}

impl Drop for Permit { fn drop(&mut self) { self.invalidate(); } }

impl Drop for Backing {
    fn drop(&mut self) {
        if self.owned {
            let layout = Layout::from_size_align(self.size, PAGE_SIZE).expect("backing layout");
            // SAFETY: the last backing reference owns this entire allocation.
            unsafe { frame::allocator().deallocate(self.base, layout) };
        }
    }
}
