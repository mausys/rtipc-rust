use nix::errno::Errno;


#[derive(Debug)]
pub enum ShmError {
    Size,
    Alignment,
}

#[derive(Debug)]
pub enum MessageError {
    Size,
    Magic,
    Version,
    Cookie,
    CachelineSize,
    AtomicSize,
}


#[derive(Debug)]
pub enum RtIpcError {
    Errno(Errno),
    Shm(ShmError),
    Message(MessageError),
    Argument

}


impl From<ShmError> for MessageError {
    fn from(_: ShmError) -> MessageError {
        MessageError::Size
    }
}

impl From<ShmError> for RtIpcError {
    fn from(e: ShmError) -> RtIpcError {
        RtIpcError::Shm(e)
    }
}

impl From<Errno> for RtIpcError {
    fn from(errno: Errno) -> RtIpcError {
        RtIpcError::Errno(errno)
    }
}



impl From<MessageError> for RtIpcError {
    fn from(field: MessageError) -> RtIpcError {
        RtIpcError::Message(field)
    }
}
