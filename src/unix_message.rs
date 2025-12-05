use nix::errno::Errno;
use nix::sys::socket::{recvmsg, sendmsg, ControlMessage, ControlMessageOwned, MsgFlags};
use nix::unistd::close;
use nix::Result;
use std::collections::VecDeque;
use std::io::{IoSlice, IoSliceMut};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::io::RawFd;

//from kernel header file net/scm.h: SCM_MAX_FD
const MAX_FD: usize = 253;

pub(crate) struct UnixMessage {
    content: Vec<u8>,
    fds: Vec<RawFd>,
    cleanup: bool,
}

impl UnixMessage {
    pub(crate) fn new(content: Vec<u8>, fds: Vec<RawFd>) -> Self {
        Self {
            content,
            fds,
            cleanup: false,
        }
    }
    pub(crate) fn send(&self, socket: RawFd) -> Result<usize> {
        let iov = [IoSlice::new(&self.content)];

        let cmsg: &[ControlMessage] = if self.fds.is_empty() {
            &[]
        } else {
            &[ControlMessage::ScmRights(self.fds.as_slice())]
        };

        sendmsg::<()>(socket, &iov, cmsg, MsgFlags::empty(), None)
    }

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

        let fds = match recv_data.cmsgs()?.next().ok_or(Errno::ENOMSG)? {
            ControlMessageOwned::ScmRights(fds) => fds,
            _ => return Err(Errno::EBADMSG),
        };

        Ok(Self {
            content,
            fds,
            cleanup: true,
        })
    }

    pub(crate) fn content(&self) -> &Vec<u8> {
        &self.content
    }

    pub(crate) fn take_fds(&mut self) -> VecDeque<OwnedFd> {
        self.fds
            .drain(0..)
            .map(|fd| unsafe { OwnedFd::from_raw_fd(fd) })
            .collect()
    }
}

impl Drop for UnixMessage {
    fn drop(&mut self) {
        if !self.cleanup {
            return;
        }
        for fd in &self.fds {
            if *fd > 0 {
                let _ = close(*fd);
            }
        }
    }
}
