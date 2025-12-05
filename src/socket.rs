use nix::errno::Errno;
use nix::sys::socket::{
    accept, bind, connect, listen, socket, AddressFamily, Backlog, SockFlag, SockType, UnixAddr,
};
use nix::unistd::unlink;
use nix::NixPath;
use std::os::fd::{OwnedFd, RawFd};
use std::os::unix::io::AsRawFd;

use crate::error::*;
use crate::protocol::{create_request, create_response, parse_request, parse_response};
use crate::unix_message::UnixMessage;
use crate::ChannelVector;
use crate::VectorParam;

pub struct Server {
    sockfd: OwnedFd,
    addr: UnixAddr,
}

impl Server {
    pub fn new<P: ?Sized + NixPath>(path: &P, backlog: Backlog) -> Result<Self, Errno> {
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

    pub fn conditional_accept<F>(&self, filter: F) -> Result<ChannelVector, ProcessRequestError>
    where
        F: Fn(&ChannelVector) -> Result<(), Errno>,
    {
        let cfd = accept(self.sockfd.as_raw_fd())?;
        let mut req = UnixMessage::receive(cfd.as_raw_fd())?;

        let result = {
            let fds = req.take_fds();
            let vparam = parse_request(req.content())?;
            let vector = ChannelVector::map(&vparam, fds)?;
            filter(&vector)?;
            Ok(vector)
        };

        let response_msg = create_response(&result.as_ref().map(|_| ()));
        let response = UnixMessage::new(response_msg, Vec::with_capacity(0));

        response.send(cfd.as_raw_fd())?;

        result
    }

    pub fn accept(&self) -> Result<ChannelVector, ProcessRequestError> {
        self.conditional_accept(|_| Ok(()))
    }
}

pub fn client_connect_fd(
    socket: RawFd,
    vparam: VectorParam,
) -> Result<ChannelVector, CreateRequestError> {
    let (vec, fds) = ChannelVector::new(&vparam)?;
    let req_msg = create_request(&vparam);
    let req = UnixMessage::new(req_msg, fds);

    req.send(socket)?;

    Ok(vec)
}

pub fn client_connect<P: ?Sized + NixPath>(
    path: &P,
    vparam: VectorParam,
) -> Result<ChannelVector, CreateRequestError> {
    let sockfd = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::empty(),
        None,
    )?;

    let addr = UnixAddr::new(path)?;

    connect(sockfd.as_raw_fd(), &addr)?;

    let (vec, fds) = ChannelVector::new(&vparam)?;

    let req_msg = create_request(&vparam);
    let req = UnixMessage::new(req_msg, fds);

    req.send(sockfd.as_raw_fd())?;

    let response = UnixMessage::receive(sockfd.as_raw_fd())?;

    parse_response(response.content().as_slice())?;

    Ok(vec)
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(path) = self.addr.path() {
            let _ = unlink(path);
        }
    }
}
