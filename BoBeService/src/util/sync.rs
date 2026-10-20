use std::sync::{Mutex, MutexGuard};

pub(crate) fn lock<'a, T>(mutex: &'a Mutex<T>, name: &'static str) -> MutexGuard<'a, T> {
    mutex.lock().unwrap_or_else(|poisoned| {
        tracing::error!(mutex = name, "sync.mutex_poisoned");
        poisoned.into_inner()
    })
}
