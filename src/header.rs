use std::mem::size_of;

use crate::cache::max_cacheline_size;
use crate::error::*;
use crate::Index;

const RTIC_MAGIC: u16 = 0x1f0c;
const RTIC_VERSION: u16 = 1;

#[repr(C)]
struct Header {
    magic: u16,
    version: u16,
    cacheline_size: u16,
    atomic_size: u16,
}

pub const HEADER_SIZE: usize = size_of::<Header>();

pub(crate) fn verify_header(buf: &[u8]) -> Result<(), MessageError> {
    if buf.len() < size_of::<Header>() {
        return Err(MessageError::Size);
    }

    let cacheline_size: u16 = max_cacheline_size().try_into().unwrap();
    let atomic_size: u16 = std::mem::size_of::<Index>().try_into().unwrap();
    let ptr: *const Header = buf.as_ptr() as *const Header;

    let header = unsafe { ptr.read() };

    if header.magic != RTIC_MAGIC {
        return Err(MessageError::Magic);
    }

    if header.version != RTIC_VERSION {
        return Err(MessageError::Version);
    }

    if header.cacheline_size != cacheline_size {
        return Err(MessageError::CachelineSize);
    }

    if header.atomic_size != atomic_size {
        return Err(MessageError::AtomicSize);
    }

    Ok(())
}

pub(crate) fn write_header(buf: &mut [u8]) {
    if buf.len() < size_of::<Header>() {
        return;
    }

    let cacheline_size: u16 = max_cacheline_size().try_into().unwrap();
    let atomic_size: u16 = std::mem::size_of::<Index>().try_into().unwrap();

    let header = Header {
        magic: RTIC_MAGIC,
        version: RTIC_VERSION,
        cacheline_size,
        atomic_size,
    };

    let ptr: *mut Header = buf.as_ptr() as *mut Header;

    unsafe {
        std::ptr::write(ptr, header);
    };
}
