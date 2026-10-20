#[path = "spin_lock.rs"]
pub mod sync;
#[path = "instant.rs"]
pub mod clock;
#[path = "hart_id.rs"]
pub mod hart;

pub mod stop {
    pub fn hush() {
        std::thread::yield_now();
    }
}
