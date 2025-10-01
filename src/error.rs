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
pub enum RtipcError {
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

impl From<ShmError> for RtipcError {
    fn from(e: ShmError) -> RtipcError {
        RtipcError::Shm(e)
    }
}

impl From<Errno> for RtipcError {
    fn from(errno: Errno) -> RtipcError {
        RtipcError::Errno(errno)
    }
}



impl From<MessageError> for RtipcError {
    fn from(field: MessageError) -> RtipcError {
        RtipcError::Message(field)
    }
}
