use std::sync::atomic::{AtomicUsize, Ordering};

use crate::log::*;

pub(crate) fn max_cacheline_size() -> usize {
    static CLS: AtomicUsize = AtomicUsize::new(0);

    let mut cls = CLS.load(Ordering::Relaxed);

    if cls != 0 {
        return cls;
    }

    let cls_str = env!("CACHELINE_SIZE");
    cls = cls_str.parse::<usize>().unwrap();

    CLS.store(cls, Ordering::Relaxed);

    info!("cache line size = {cls}");
    cls
}
