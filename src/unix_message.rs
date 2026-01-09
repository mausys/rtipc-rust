use nix::errno::Errno;
use nix::sys::socket::{recvmsg, sendmsg, ControlMessage, ControlMessageOwned, MsgFlags};
use nix::unistd::close;
use nix::Result;
use std::collections::VecDeque;
use std::io::{IoSlice, IoSliceMut};
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::os::unix::io::RawFd;

//from kernel header file net/scm.h: SCM_MAX_FD
const MAX_FD: usize = 253;

pub(crate) struct UnixMessageTx<'a> {
    content: Vec<u8>,
    fds: Vec<BorrowedFd<'a>>,
}

pub(crate) struct UnixMessageRx {
    content: Vec<u8>,
    fds: Vec<OwnedFd>,
}

impl<'a> UnixMessageTx<'a> {
    pub(crate) fn new(content: Vec<u8>, fds: Vec<BorrowedFd<'a>>) -> Self {
        Self { content, fds }
    }

    pub(crate) fn send(&self, socket: RawFd) -> Result<usize> {
        let iov = [IoSlice::new(&self.content)];
        let fds: Vec::<RawFd> = self.fds.iter().map(|fd| fd.as_raw_fd()).collect();

        let cmsg: &[ControlMessage] = &[ControlMessage::ScmRights(fds.as_slice())];

        sendmsg::<()>(socket, &iov, cmsg, MsgFlags::empty(), None)
    }
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
                ControlMessageOwned::ScmRights(fds) => {
                    Ok(fds.iter().map(|fd| unsafe { OwnedFd::from_raw_fd(*fd) }).collect())
                }
                _ => return Err(Errno::EBADMSG),
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
