#![cfg(unix)]

use std::{
    fmt,
    mem::size_of,
    num::NonZeroUsize,
    os::fd::{BorrowedFd, OwnedFd, AsFd},
    ptr::NonNull,
    sync::{Arc, Weak},
};

use nix::{
    errno::Errno,
    fcntl::{fcntl, SealFlag, F_ADD_SEALS},
    libc::c_void,
    sys::{
        memfd::{memfd_create, MFdFlags},
        mman::{mlock, mmap, munmap, MapFlags, ProtFlags},
        stat::fstat,
    },
    unistd::ftruncate,
};

use crate::error::*;
use crate::log::*;

#[derive(Debug, Copy, Clone)]
pub(crate) struct Span {
    pub offset: usize,
    pub size: NonZeroUsize,
}

pub(crate) struct Chunk {
    shm: Arc<SharedMemory>,
    offset: usize,
    size: NonZeroUsize,
}

impl Chunk {
    pub(crate) fn get_ptr<T>(&self, offset: usize) -> Result<*mut T, ShmPointerError> {
        let size = NonZeroUsize::new(size_of::<T>()).unwrap();
        let ptr = self.get_span_ptr(&Span { offset, size })?;

        Ok(ptr.cast())
    }

    pub(crate) fn get_span_ptr(&self, span: &Span) -> Result<*mut (), ShmPointerError> {
        if span.offset + span.size.get() > self.size.get() {
            return Err(ShmPointerError::OutOfBounds);
        }

        let ptr: *mut () = unsafe { self.shm.ptr.byte_add(self.offset + span.offset) };

        Ok(ptr)
    }
}

#[derive(Debug)]
pub struct SharedMemory {
    fd: OwnedFd,
    me: Weak<Self>,
    ptr: *mut (),
    size: NonZeroUsize,
}

impl SharedMemory {
    pub fn alloc(&self, offset: usize, size: NonZeroUsize) -> Result<Chunk, ShmPointerError> {
        if offset + size.get() > self.size.get() {
            return Err(ShmPointerError::OutOfBounds);
        }

        Ok(Chunk {
            shm: self.me.upgrade().unwrap(),
            offset,
            size,
        })
    }

    fn init<F: std::os::fd::AsFd>(fd: F, size: NonZeroUsize) -> Result<(), Errno> {
        ftruncate(&fd, size.get() as i64)?;
        fcntl(
            &fd,
            F_ADD_SEALS(SealFlag::F_SEAL_GROW | SealFlag::F_SEAL_SHRINK | SealFlag::F_SEAL_SEAL),
        )?;
        Ok(())
    }

    fn create(fd: OwnedFd) -> Result<Arc<Self>, Errno> {
        let stat = fstat(&fd)?;

        let size = NonZeroUsize::new(stat.st_size as usize).ok_or(Errno::EBADFD)?;

        let ptr = unsafe {
            mmap(
                None,                                         // Desired addr
                size,                                         // size of mapping
                ProtFlags::PROT_READ | ProtFlags::PROT_WRITE, // Permissions on pages
                MapFlags::MAP_SHARED,                         // What kind of mapping
                &fd,                                          // fd
                0,                                            // Offset into fd
            )
        }?;

        unsafe {
            mlock(ptr, size.get())?;
        }

        Ok(Arc::new_cyclic(|me| Self {
            me: me.clone(),
            fd,
            ptr: ptr.as_ptr().cast(),
            size,
        }))
    }

    pub(crate) fn new(size: NonZeroUsize) -> Result<Arc<Self>, Errno> {
        let fd: OwnedFd = memfd_create("rtipc", MFdFlags::MFD_ALLOW_SEALING)?;
        Self::init(&fd, size)?;
        Self::create(fd)
    }

    pub(crate) fn from_fd(fd: OwnedFd) -> Result<Arc<Self>, Errno> {
        Self::create(fd)
    }

    pub(crate) fn fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

impl Drop for SharedMemory {
    fn drop(&mut self) {
        let ptr: NonNull<c_void> = NonNull::new(self.ptr as *mut c_void).unwrap();
        debug!("unmap {ptr:?}");
        if let Err(_e) = unsafe { munmap(ptr, self.size.get()) } {
            error!("munmap failed with : {_e}");
        }
    }
}

impl fmt::Display for SharedMemory {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "ptr: {:p}, size: {}", self.ptr, self.size)
    }
}
