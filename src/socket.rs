use std::os::fd::{OwnedFd, RawFd, AsFd};
use nix::sys::socket::{socket, AddressFamily, SockType, SockFlag, UnixAddr, bind, listen, Backlog, accept};
use std::os::unix::io::AsRawFd;
use nix::NixPath;

use crate::ChannelVector;
use crate::request::Request;
use crate::error::*;
use crate::ChannelParam;
use crate::protocol::create_request_message;

struct Server{
  sockfd: OwnedFd,
  addr: UnixAddr,
}



impl Server {
  pub fn new<P: ?Sized + NixPath>(path: &P, backlog: Backlog) -> Result<Self, RtIpcError> {
    let addr = UnixAddr::new(path)?;
    let sockfd = socket(AddressFamily::Unix, SockType::SeqPacket, SockFlag::empty(), None)?;
    bind(sockfd.as_raw_fd(), &addr)?;
    listen(&sockfd, backlog)?;
    Ok(Self{sockfd, addr})
  }

  pub fn accept(&self) -> Result<ChannelVector, RtIpcError> {
    let cfd =  accept(self.sockfd.as_raw_fd())?;
    let req = Request::receive(cfd.as_raw_fd())?;
    ChannelVector::from_request(req)
  }
}


pub fn client_connect_fd(socket: RawFd, producer_params: &Vec<ChannelParam>,
  consumer_params: &Vec<ChannelParam>,
  info: Vec<u8>) -> Result<ChannelVector, RtIpcError> {
  let msg = create_request_message(producer_params, consumer_params, &info);

  let (vec, fds) = ChannelVector::new(producer_params, consumer_params, info)?;

  let req = Request::new(msg, fds);

  req.send(socket)?;

  Ok(vec)
}
