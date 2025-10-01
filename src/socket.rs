use nix::sys::socket::{
    accept, bind, listen, socket, AddressFamily, Backlog, SockFlag, SockType, UnixAddr,
};
use nix::NixPath;
use std::os::fd::{OwnedFd, RawFd};
use std::os::unix::io::AsRawFd;

use crate::error::*;
use crate::request::Request;
use crate::{ChannelParam, VectorParam};
use crate::ChannelVector;

struct Server {
    sockfd: OwnedFd,
    addr: UnixAddr,
}

impl Server {
    pub fn new<P: ?Sized + NixPath>(path: &P, backlog: Backlog) -> Result<Self, RtIpcError> {
        let addr = UnixAddr::new(path)?;
        let sockfd = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::empty(),
            None,
        )?;
        bind(sockfd.as_raw_fd(), &addr)?;
        listen(&sockfd, backlog)?;
        Ok(Self { sockfd, addr })
    }

    pub fn accept(&self) -> Result<ChannelVector, RtIpcError> {
        let cfd = accept(self.sockfd.as_raw_fd())?;
        let req = Request::receive(cfd.as_raw_fd())?;
        ChannelVector::from_request(req)
    }
}

pub fn client_connect_fd(
    socket: RawFd,
    vparam: VectorParam
) -> Result<ChannelVector, RtIpcError> {

    let (vec, req) = ChannelVector::new(&vparam)?;

    req.send(socket)?;

    Ok(vec)
}
