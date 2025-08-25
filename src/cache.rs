use cache_size::{cache_line_size, CacheType};

use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) fn max_cacheline_size() -> usize {
    static CLS: AtomicUsize = AtomicUsize::new(0);

    let mut cls = CLS.load(Ordering::Relaxed);

    if cls != 0 {
        return cls;
    }

    // TODO: replace this with max_align_t
    cls = std::mem::align_of::<f64>();

    for level in 1..=2 {
        cls = match cache_line_size(level, CacheType::Data) {
            None => cls,
            Some(s) => {
                if s > cls {
                    s
                } else {
                    cls
                }
            }
        };
    }

    CLS.store(cls, Ordering::Relaxed);
    cls
}

fn mem_align(size: usize, alignment: usize) -> usize {
    (size + alignment - 1) & !(alignment - 1)
}

pub(crate) fn cacheline_aligned(size: usize) -> usize {
    mem_align(size, max_cacheline_size())
}
