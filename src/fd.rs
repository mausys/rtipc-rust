use std::os::unix::io::{RawFd, OwnedFd};

use nix::fcntl::readlink;

use nix::errno::Errno;
use nix::sys::eventfd::EfdFlags;
use nix::sys::eventfd::EventFd;

use nix::Result;

const PROC_SELF_FD: &str = "/proc/self/fd/";

pub(crate) fn eventfd() ->  Result<OwnedFd> {
  let evd = EventFd::from_flags(EfdFlags::EFD_CLOEXEC | EfdFlags::EFD_SEMAPHORE | EfdFlags::EFD_NONBLOCK)?;
  Ok(OwnedFd::from(evd))
}

fn fd_link(fd: RawFd) ->  Result<String> {
    let path = format!("{}{}", PROC_SELF_FD, fd);
    let oslink = readlink( path.as_str())?;
    let link = oslink.to_str().ok_or(Errno::EBADF)?.to_owned();
    Ok(link)
}


pub(crate) fn check_eventfd(fd: RawFd) -> Result<()>
{
  let expected =  "anon_inode:[eventfd";

  let link = fd_link(fd)?;

  if link.get(0..expected.len()).ok_or(Errno::EBADF)? != expected {
      return Err(Errno::EBADF);
  }

  Ok(())
}
