use std::fs::read_to_string;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::log::*;
use crate::mem_align;

#[cfg(not(feature = "predefined_cacheline_size"))]
#[derive(Debug, PartialEq, Eq)]
enum CacheType {
    Data,
    Instruction,
    Unified,
}

#[cfg(not(feature = "predefined_cacheline_size"))]
struct Cache {
    level: usize,
    cls: usize,
    cache_type: CacheType,
}

#[cfg(not(feature = "predefined_cacheline_size"))]
fn get_cache_attr_path(cpu: usize, index: usize, attr: &str) -> PathBuf {
    PathBuf::from(format!(
        "/sys/devices/system/cpu/cpu{cpu}/cache/index{index}/{attr}"
    ))
}

#[cfg(not(feature = "predefined_cacheline_size"))]
fn cache_read_attr(cpu: usize, index: usize, attr: &str) -> Result<usize, std::io::Error> {
    read_to_string(get_cache_attr_path(cpu, index, attr))?
        .trim_end()
        .parse::<usize>()
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "parser error"))
}

#[cfg(not(feature = "predefined_cacheline_size"))]
fn cache_read_type(cpu: usize, index: usize) -> Result<CacheType, std::io::Error> {
    let strtype = read_to_string(get_cache_attr_path(cpu, index, "type"))?;
    match strtype.trim_end() {
        "Data" => Ok(CacheType::Data),
        "Instruction" => Ok(CacheType::Instruction),
        "Unified" => Ok(CacheType::Unified),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "parser error",
        )),
    }
}

#[cfg(not(feature = "predefined_cacheline_size"))]
fn read_cache(cpu: usize, index: usize) -> Result<Cache, std::io::Error> {
    let level = cache_read_attr(cpu, index, "level")?;
    let cls = cache_read_attr(cpu, index, "coherency_line_size")?;
    let cache_type = cache_read_type(cpu, index)?;

    debug!("cache on cpu[{cpu}]: level={level} cls={cls}, type={cache_type:?}",);

    Ok(Cache {
        level,
        cls,
        cache_type,
    })
}

#[cfg(not(feature = "predefined_cacheline_size"))]
pub(crate) fn max_cacheline_size() -> usize {
    static CLS: AtomicUsize = AtomicUsize::new(0);

    let mut cls = CLS.load(Ordering::Relaxed);

    if cls != 0 {
        return cls;
    }

    // TODO: replace this with max_align_t
    cls = std::mem::align_of::<f64>();

    for index in 0..4 {
        if let Ok(cache) = read_cache(0, index) {
            if cache.cache_type != CacheType::Data {
                continue;
            }
            if cache.level > 2 {
                continue;
            }
            if cache.cls > cls {
                cls = cache.cls;
            }
        }
    }

    CLS.store(cls, Ordering::Relaxed);
    info!("cache line size = {cls}");
    cls
}

#[cfg(feature = "predefined_cacheline_size")]
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

pub(crate) fn cacheline_aligned(size: usize) -> usize {
    mem_align(size, max_cacheline_size())
}
