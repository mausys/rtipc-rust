use crate::cache::max_cacheline_size;
use crate::shm::Chunk;
use crate::error::*;
use crate::Index;

const RTIC_MAGIC: u16 = 0x1f0c;
const RTIC_VERSION: u16 = 1;

#[derive(Copy, Clone)]
#[repr(C)]
pub struct Header {
    magic: u16,
    version: u16,
    cookie: u32,
    /**< cookie for object protocol */
    pub num_channels: [u32; 2],
    /**< number of channels for producers / consumers */
    cacheline_size: u16,
    atomic_size: u16,
}

impl Header {
    pub(crate) fn new(num_consumers: u32, num_producers: u32, cookie: u32) -> Self {
        let cacheline_size: u16 = max_cacheline_size().try_into().unwrap();
        let atomic_size: u16 = std::mem::size_of::<Index>().try_into().unwrap();
        Header {
            magic: RTIC_MAGIC,
            version: RTIC_VERSION,
            cookie,
            num_channels: [num_consumers, num_producers],
            cacheline_size,
            atomic_size,
        }
    }

    pub(crate) fn from_chunk(chunk: &Chunk, cookie: u32) -> Result<Header, HeaderError> {
        let cacheline_size: u16 = max_cacheline_size().try_into().unwrap();
        let atomic_size: u16 = std::mem::size_of::<Index>().try_into().unwrap();
        let ptr: *const Header = chunk.get_ptr(0)?;

        let header = unsafe { ptr.read() };

        if header.magic != RTIC_MAGIC {
            return Err(HeaderError::Magic);
        }

        if header.version != RTIC_VERSION {
            return Err(HeaderError::Version);
        }
        if header.cookie != cookie {
            return Err(HeaderError::Cookie);
        }

        if header.cacheline_size != cacheline_size {
            return Err(HeaderError::CachelineSize);
        }

        if header.atomic_size != atomic_size {
            return Err(HeaderError::AtomicSize);
        }

        Ok(header)
    }

    pub(crate) fn write(&self, chunk: &Chunk) -> Result<(), MemError> {
        let ptr: *mut Header = chunk.get_ptr(0)?;

        unsafe {
            std::ptr::write(ptr, *self);
        }

        Ok(())
    }
}
