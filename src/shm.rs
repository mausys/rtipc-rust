#![cfg(unix)]

use std::{
    fmt,
    mem::size_of,
    num::NonZeroUsize,
    os::fd::OwnedFd,
    path::{Path, PathBuf},
    ptr::NonNull,
    sync::{Arc, Weak},
};

use nix::{
    errno::Errno,
    fcntl::{fcntl, OFlag, SealFlag, F_ADD_SEALS},
    libc::c_void,
    sys::{
        memfd::{memfd_create, MFdFlags},
        mman::{mmap, munmap, shm_open, shm_unlink, MapFlags, ProtFlags},
        stat::{fstat, Mode},
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
    _shm: Arc<SharedMemory>,
    ptr: *mut (),
    size: NonZeroUsize,
}

impl Chunk {
    pub(crate) fn get_ptr<T>(&self, offset: usize) -> Result<*mut T, MemError> {
        let size = NonZeroUsize::new(size_of::<T>()).unwrap();
        let ptr = self.get_span_ptr(&Span { offset, size })?;

        Ok(ptr.cast())
    }

    pub(crate) fn get_span_ptr(&self, span: &Span) -> Result<*mut (), MemError> {
        if span.offset + span.size.get() > self.size.get() {
            return Err(MemError::Size);
        }

        let ptr: *mut () = unsafe { self.ptr.byte_add(span.offset) };

        Ok(ptr)
    }
}

#[derive(Debug)]
pub struct SharedMemory {
    fd: OwnedFd,
    me: Weak<SharedMemory>,
    ptr: *mut (),
    size: NonZeroUsize,
    path: Option<PathBuf>,
}

impl SharedMemory {
    pub fn alloc(&self, span: &Span) -> Result<Chunk, MemError> {
        if span.offset + span.size.get() > self.size.get() {
            return Err(MemError::Size);
        }

        let ptr: *mut () = unsafe { self.ptr.byte_add(span.offset) };

        Ok(Chunk {
            _shm: self.me.upgrade().unwrap(),
            ptr,
            size: span.size,
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

    fn new(fd: OwnedFd, path: Option<PathBuf>) -> Result<Arc<SharedMemory>, Errno> {
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
        Ok(Arc::new_cyclic(|me| SharedMemory {
            me: me.clone(),
            fd,
            ptr: ptr.as_ptr().cast(),
            size,
            path,
        }))
    }

    pub(crate) fn new_anon(size: NonZeroUsize) -> Result<Arc<SharedMemory>, Errno> {
        let fd: OwnedFd = memfd_create("test", MFdFlags::MFD_ALLOW_SEALING)?;
        SharedMemory::init(&fd, size)?;
        SharedMemory::new(fd, None)
    }

    pub(crate) fn new_named(
        size: NonZeroUsize,
        path: &Path,
        mode: Mode,
    ) -> Result<Arc<SharedMemory>, Errno> {
        let fd: OwnedFd = shm_open(path, OFlag::O_CREAT | OFlag::O_EXCL | OFlag::O_RDWR, mode)?;
        let path_buf: PathBuf = PathBuf::from(path);
        SharedMemory::init(&fd, size)?;
        SharedMemory::new(fd, Some(path_buf))
    }

    pub(crate) fn from_fd(fd: OwnedFd) -> Result<Arc<SharedMemory>, Errno> {
        SharedMemory::new(fd, None)
    }

    pub(crate) fn get_fd(&self) -> &OwnedFd {
        &self.fd
    }
}

impl Drop for SharedMemory {
    fn drop(&mut self) {
        let ptr: NonNull<c_void> = NonNull::new(self.ptr as *mut c_void).unwrap();
        if let Err(_e) = unsafe { munmap(ptr, self.size.get()) } {
            error!("munmap failed with : {_e}");
        }
        let path = &self.path;
        if let Some(path) = path {
            if let Err(_e) = shm_unlink(path) {
                error!("munmap failed with : {_e}");
            }
        };
    }
}

impl fmt::Display for SharedMemory {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let path = &self.path;
        if let Some(path) = path {
            write!(
                f,
                "ptr: {:p}, size: {} path: {}",
                self.ptr,
                self.size,
                path.display()
            )
        } else {
            write!(f, "ptr: {:p}, size: {}", self.ptr, self.size)
        }
    }
}
