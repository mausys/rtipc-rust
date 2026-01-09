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
use crate::unix_message::{UnixMessageRx, UnixMessageTx};
use crate::{ChannelVector, VectorConfig, ChannelIn, VectorIn};

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
        F: Fn(&ChannelVector) -> bool,
    {
        let cfd = accept(self.sockfd.as_raw_fd())?;
        let mut req = UnixMessageRx::receive(cfd.as_raw_fd())?;

        let mut fds = req.take_fds();
        let vconfig = parse_request(req.content())?;

        let shmfd = fds.pop_front().ok_or(ProcessRequestError::MissingFileDescriptor)?;

        let mut consumers = Vec::<ChannelIn>::new();

        for config in vconfig.consumers {
            let channel = if config.eventfd {
                let fd = fds.pop_front().ok_or(ProcessRequestError::MissingFileDescriptor)?;
                ChannelIn{queue: config.queue, eventfd: Some(fd)}
            } else {
                ChannelIn{queue: config.queue, eventfd: None}
            };
            consumers.push(channel);
        }

        let mut producers = Vec::<ChannelIn>::new();

        for config in vconfig.producers {
            let channel = if config.eventfd {
            let fd = fds.pop_front().ok_or(ProcessRequestError::MissingFileDescriptor)?;
                ChannelIn{queue: config.queue, eventfd: Some(fd)}
            } else {
                ChannelIn{queue: config.queue, eventfd: None}
            };
            producers.push(channel);
        }

        let vin = VectorIn {
            consumers,
            producers,
            info: vconfig.info,
            shmfd,
        };

        let result = {
            let vector = ChannelVector::map(vin)?;
            if !filter(&vector) {
                Err(ProcessRequestError::ResponseError)
            } else {
                Ok(vector)
            }
        };

        let response_msg = create_response(&result.as_ref().map(|_| ()));

        let response = UnixMessageTx::new(response_msg, Vec::with_capacity(0));

        response.send(cfd.as_raw_fd())?;

        result
    }

    pub fn accept(&self) -> Result<ChannelVector, ProcessRequestError> {
        self.conditional_accept(|_| true)
    }
}

pub fn client_connect_fd(
    socket: RawFd,
    vconfig: VectorConfig,
) -> Result<ChannelVector, CreateRequestError> {
    let vec = ChannelVector::new(&vconfig)?;
    let req_msg = create_request(&vconfig);
    let fds = vec.collect_fds();

    let req = UnixMessageTx::new(req_msg, fds);

    req.send(socket)?;

    let response = UnixMessageRx::receive(socket.as_raw_fd())?;

    parse_response(response.content().as_slice())?;

    Ok(vec)
}

pub fn client_connect<P: ?Sized + NixPath>(
    path: &P,
    vconfig: VectorConfig,
) -> Result<ChannelVector, CreateRequestError> {
    let socket = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::empty(),
        None,
    )?;

    let addr = UnixAddr::new(path)?;

    connect(socket.as_raw_fd(), &addr)?;

    let vec = ChannelVector::new(&vconfig)?;

    let req_msg = create_request(&vconfig);
    let fds = vec.collect_fds();

    let req = UnixMessageTx::new(req_msg, fds);

    req.send(socket.as_raw_fd())?;

    let response = UnixMessageRx::receive(socket.as_raw_fd())?;

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
