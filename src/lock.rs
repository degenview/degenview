//! Locks taken whatever another thread did while holding them. A thread that panics with a lock
//! held poisons it; every lock in the app is then taken as it is rather than spreading that panic
//! to each thread that touches the same state (the panic itself is already reported by
//! `crash_report`). Code that treats a poisoned lock as "unavailable" (`if let Ok(..) = m.lock()`)
//! says so where it locks.

use std::sync::{Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

pub fn lock<T: ?Sized>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

pub fn read<T: ?Sized>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

pub fn write<T: ?Sized>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_poisoned_lock_is_still_taken() {
        let mutex = Mutex::new(1);
        let rw = RwLock::new(2);
        std::thread::scope(|scope| {
            let _ = scope
                .spawn(|| {
                    let _held = (mutex.lock().unwrap(), rw.write().unwrap());
                    panic!("poisons both locks");
                })
                .join();
        });
        assert!(mutex.is_poisoned() && rw.is_poisoned());
        *lock(&mutex) += 1;
        *write(&rw) += 1;
        assert_eq!((*lock(&mutex), *read(&rw)), (2, 3));
    }
}
