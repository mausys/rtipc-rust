use std::fmt;

use std::os::fd::BorrowedFd;
use std::time::Duration;

use nix::errno::Errno;
use nix::poll::{poll, PollFd, PollFlags, PollTimeout};

#[repr(u32)]
#[derive(Copy, Clone, Debug)]
pub enum CommandId {
    Hello = 1,
    Stop = 2,
    SendEvent = 3,
    Div = 4,
}

#[derive(Copy, Clone, Debug)]
pub struct MsgCommand {
    pub id: u32,
    pub args: [i32; 3],
}

#[derive(Copy, Clone, Debug)]
pub struct MsgResponse {
    pub id: u32,
    pub result: i32,
    pub data: i32,
}

#[derive(Copy, Clone, Debug)]
pub struct MsgEvent {
    pub id: u32,
    pub nr: u32,
}

impl fmt::Display for MsgCommand {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "id: {}", self.id as u32)?;
        for (idx, arg) in self.args.iter().enumerate() {
            writeln!(f, "\targ[{}]: {}", idx, arg)?
        }
        Ok(())
    }
}

impl fmt::Display for MsgResponse {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(
            f,
            "id: {}\n\tresult: {}\n\tdata: {}",
            self.id as u32, self.result, self.data
        )
    }
}

impl fmt::Display for MsgEvent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "id: {}\n\tnr: {}", self.id, self.nr)
    }
}

pub fn wait_pollin(fd: BorrowedFd, timeout: Duration) -> Result<bool, Errno> {
    let mut fds = [PollFd::new(fd, PollFlags::POLLIN)];
    let duration: PollTimeout = timeout.try_into().unwrap();
    poll(&mut fds, duration)?;
    Ok(fds[0].revents().map_or(false, |flags| !flags.is_empty()))
}
