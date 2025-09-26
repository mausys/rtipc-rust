use std::io::{IoSlice, IoSliceMut};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::io::RawFd;

use nix::errno::Errno;
use nix::sys::socket::{recvmsg, sendmsg, ControlMessage, ControlMessageOwned, MsgFlags};
use nix::unistd::close;
use nix::Result;

//from kernel header file net/scm.h: SCM_MAX_FD
const MAX_FD: usize = 253;

pub(crate) struct Request {
    msg: Vec<u8>,
    fds: Vec<RawFd>,
}

impl Request {
    pub(crate) fn new(msg: Vec<u8>, fds: Vec<RawFd>) -> Self {
        Self { msg, fds }
    }
    pub(crate) fn send(&self, socket: RawFd) -> Result<usize> {
        let iov = [IoSlice::new(b"hello")];

        let cmsg = ControlMessage::ScmRights(self.fds.as_slice());

        sendmsg::<()>(socket, &iov, &[cmsg], MsgFlags::empty(), None)
    }

    pub(crate) fn receive(socket: RawFd) -> Result<Self> {
        let recv_empty = recvmsg::<()>(
            socket,
            &mut [] as &mut [IoSliceMut],
            //iov.as_mut_slice(),
            None,
            MsgFlags::union(MsgFlags::MSG_PEEK, MsgFlags::MSG_TRUNC),
        )?;

        if recv_empty.bytes == 0 {
            return Err(Errno::EPROTO);
        }

        let mut msg: Vec<u8> = vec![0; recv_empty.bytes];
        let mut iov = [IoSliceMut::new(msg.as_mut_slice())];
        let mut cmsg = cmsg_space!([RawFd; MAX_FD]);

        let recv_data = recvmsg::<()>(
            socket,
            &mut iov,
            Some(&mut cmsg),
            MsgFlags::union(MsgFlags::MSG_PEEK, MsgFlags::MSG_TRUNC),
        )?;

        let fds = match recv_data.cmsgs()?.next().ok_or(Errno::EPROTO)? {
            ControlMessageOwned::ScmRights(fds) => fds,
            _ => return Err(Errno::EPROTO),
        };

        Ok(Self { msg, fds })
    }

    pub(crate) fn msg(&self) -> &Vec<u8> {
        &self.msg
    }

    pub(crate) fn take_fd(&mut self, index: usize) -> Option<OwnedFd> {
        if let Some(fd) = self.fds.get_mut(index) {
            if *fd < 0 {
                None
            } else {
                let owned_fd = unsafe { OwnedFd::from_raw_fd(*fd) };
                *fd = -1;
                Some(owned_fd)
            }
        } else {
            None
        }
    }

    pub(crate) fn add_fd(&mut self, fd: RawFd) -> Result<()> {
        if self.fds.len() >= MAX_FD {
            return Err(Errno::ENOMEM);
        }
        self.fds.push(fd);
        Ok(())
    }
}

impl Drop for Request {
    fn drop(&mut self) {
        for fd in &self.fds {
            if *fd > 0 {
                let _ = close(*fd);
            }
        }
    }
}
