use std::collections::VecDeque;
use std::io::{IoSlice, IoSliceMut};
use std::num::NonZeroUsize;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::io::RawFd;

use nix::{
    errno::Errno,
    fcntl::{fcntl, readlink, SealFlag, F_ADD_SEALS},
    sys::{
        eventfd::{EfdFlags, EventFd},
        memfd::{memfd_create, MFdFlags},
        socket::{recvmsg, sendmsg, ControlMessage, ControlMessageOwned, MsgFlags},
    },
    unistd::ftruncate,
    Result,
};

use crate::log::*;

//from kernel header file net/scm.h: SCM_MAX_FD
const MAX_FD: usize = 253;

const PROC_SELF_FD: &str = "/proc/self/fd/";

pub fn shmfd_create(size: NonZeroUsize) -> Result<OwnedFd> {
    let fd: OwnedFd = memfd_create("rtipc", MFdFlags::MFD_ALLOW_SEALING)?;
    ftruncate(&fd, size.get() as i64)?;
    fcntl(
        &fd,
        F_ADD_SEALS(SealFlag::F_SEAL_GROW | SealFlag::F_SEAL_SHRINK | SealFlag::F_SEAL_SEAL),
    )?;
    Ok(fd)
}

pub(crate) fn eventfd_create() -> Result<EventFd> {
    let evd = EventFd::from_flags(
        EfdFlags::EFD_CLOEXEC | EfdFlags::EFD_SEMAPHORE | EfdFlags::EFD_NONBLOCK,
    )
    .inspect_err(|e| error!("eventfd failed {e:?}"))?;
    Ok(evd)
}

fn fd_link(fd: RawFd) -> Result<String> {
    let path = format!("{PROC_SELF_FD}{fd}");
    let oslink = readlink(path.as_str()).inspect_err(|e| error!("readlink failed {e:?}"))?;
    let link = oslink
        .to_str()
        .ok_or(Errno::EBADF)
        .inspect_err(|_| error!("oslink.to_str failed"))?
        .to_owned();
    Ok(link)
}

pub(crate) fn into_eventfd(fd: OwnedFd) -> Result<EventFd> {
    let expected = "anon_inode:[eventfd";

    let link = fd_link(fd.as_raw_fd())?;

    if link.get(0..expected.len()).ok_or(Errno::EBADF)? != expected {
        error!("link is not eventfd {link:?}");
        return Err(Errno::EBADF);
    }

    let efd = unsafe { EventFd::from_owned_fd(fd) };

    Ok(efd)
}

pub(crate) fn check_memfd(fd: BorrowedFd<'_>) -> Result<()> {
    let expected = "/memfd:";

    let link = fd_link(fd.as_raw_fd())?;

    if link.get(0..expected.len()).ok_or(Errno::EBADF)? != expected {
        error!("link is not memfd {link:?}");
        Err(Errno::EBADF)
    } else {
        Ok(())
    }
}

pub(crate) struct UnixMessageTx<'a> {
    content: Vec<u8>,
    fds: Vec<BorrowedFd<'a>>,
}

impl<'a> UnixMessageTx<'a> {
    pub(crate) fn new(content: Vec<u8>, fds: Vec<BorrowedFd<'a>>) -> Self {
        Self { content, fds }
    }

    pub(crate) fn send(&self, socket: RawFd) -> Result<usize> {
        let iov = [IoSlice::new(&self.content)];
        let fds: Vec<RawFd> = self.fds.iter().map(|fd| fd.as_raw_fd()).collect();

        let cmsg: &[ControlMessage] = &[ControlMessage::ScmRights(fds.as_slice())];

        sendmsg::<()>(socket, &iov, cmsg, MsgFlags::empty(), None)
    }
}

pub(crate) struct UnixMessageRx {
    content: Vec<u8>,
    fds: Vec<OwnedFd>,
}

impl UnixMessageRx {
    pub(crate) fn receive(socket: RawFd) -> Result<Self> {
        let recv_empty = recvmsg::<()>(
            socket,
            &mut [] as &mut [IoSliceMut],
            None,
            MsgFlags::union(MsgFlags::MSG_PEEK, MsgFlags::MSG_TRUNC),
        )?;

        if recv_empty.bytes == 0 {
            return Err(Errno::ENOMSG);
        }

        let mut content: Vec<u8> = vec![0; recv_empty.bytes];
        let mut iov = [IoSliceMut::new(content.as_mut_slice())];
        let mut cmsg = cmsg_space!([RawFd; MAX_FD]);

        let recv_data = recvmsg::<()>(
            socket,
            &mut iov,
            Some(&mut cmsg),
            MsgFlags::union(MsgFlags::MSG_PEEK, MsgFlags::MSG_TRUNC),
        )?;

        let fds = recv_data.cmsgs()?.next().map_or_else(
            || Ok(Vec::with_capacity(0)),
            |fds| match fds {
                ControlMessageOwned::ScmRights(fds) => Ok(fds
                    .iter()
                    .map(|fd| unsafe { OwnedFd::from_raw_fd(*fd) })
                    .collect()),
                _ => Err(Errno::EBADMSG),
            },
        )?;

        Ok(Self { content, fds })
    }

    pub(crate) fn content(&self) -> &Vec<u8> {
        &self.content
    }

    pub(crate) fn take_fds(&mut self) -> VecDeque<OwnedFd> {
        self.fds.drain(0..).collect()
    }
}
