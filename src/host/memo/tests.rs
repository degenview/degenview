use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn a_value_is_kept_while_its_key_holds() {
    let mut memo: Memo<(u64, i64), String> = Memo::default();
    let made = AtomicUsize::new(0);
    let mut get = |key| {
        memo.get_or(key, || {
            made.fetch_add(1, Ordering::AcqRel);
            format!("{key:?}")
        })
    };
    let first = get((1, 10));
    assert!(Arc::ptr_eq(&first, &get((1, 10))));
    let second = get((1, 11));
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(*second, "(1, 11)");
    assert_eq!(made.load(Ordering::Acquire), 2);
}
