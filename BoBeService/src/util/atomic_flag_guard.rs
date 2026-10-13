use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) struct AtomicFlagGuard {
    flag: Arc<AtomicBool>,
}

impl AtomicFlagGuard {
    pub(crate) fn try_acquire(flag: Arc<AtomicBool>) -> Option<Self> {
        if flag
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return None;
        }
        Some(Self { flag })
    }
}

impl Drop for AtomicFlagGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::Release);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests panic on precondition failures")]
mod tests {
    use super::*;

    #[test]
    fn acquisition_remains_exclusive_and_unwind_releases_ownership() {
        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = Arc::clone(&flag);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _guard = AtomicFlagGuard::try_acquire(Arc::clone(&flag_clone)).unwrap();
            assert!(AtomicFlagGuard::try_acquire(flag_clone).is_none());
            panic!("simulated failure");
        }));
        assert!(result.is_err());
        assert!(!flag.load(Ordering::Acquire));
        let guard = AtomicFlagGuard::try_acquire(Arc::clone(&flag)).unwrap();
        drop(guard);
        assert!(!flag.load(Ordering::Acquire));
    }
}
