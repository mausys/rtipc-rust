use std::mem::size_of;

use crate::cache::max_cacheline_size;
use crate::error::*;
use crate::Index;

const RTIC_MAGIC: u16 = 0x1f0c;
const RTIC_VERSION: u16 = 1;

#[derive(Copy, Clone)]
#[repr(C)]
pub struct Header {
    magic: u16,
    version: u16,
    /**< cookie for object protocol */
    cacheline_size: u16,
    atomic_size: u16,
}


pub(crate) fn check_header(buf: &[u8; size_of::<Header>()]) -> Result<(), HeaderError> {
    let cacheline_size: u16 = max_cacheline_size().try_into().unwrap();
    let atomic_size: u16 = std::mem::size_of::<Index>().try_into().unwrap();
    let ptr: *const Header = buf.as_ptr() as  *const Header;

    let header = unsafe { ptr.read() };

    if header.magic != RTIC_MAGIC {
        return Err(HeaderError::Magic);
    }

    if header.version != RTIC_VERSION {
        return Err(HeaderError::Version);
    }

    if header.cacheline_size != cacheline_size {
        return Err(HeaderError::CachelineSize);
    }

    if header.atomic_size != atomic_size {
        return Err(HeaderError::AtomicSize);
    }

    Ok(())
}

pub(crate) fn write_header(buf: &mut [u8; size_of::<Header>()]) {

    let cacheline_size: u16 = max_cacheline_size().try_into().unwrap();
    let atomic_size: u16 = std::mem::size_of::<Index>().try_into().unwrap();

    let header =  Header {
        magic: RTIC_MAGIC,
        version: RTIC_VERSION,
        cacheline_size,
        atomic_size,
    };

    let ptr: *mut Header = buf.as_ptr() as  *mut Header;


    unsafe {
        std::ptr::write(ptr, header);
    };
}
