use nix::errno::Errno;

#[derive(Debug)]
pub enum CreateError {
    Argument,
    Mem(MemError),
    Header(HeaderError),
    Errno(Errno),
    Unknown(u32),
}

#[derive(Debug)]
pub enum ChannelError {
    Index,
    Mem(MemError)
}

#[derive(Debug)]
pub enum MemError {
    Size,
    Alignment,
    Value,
    Index,
}

#[derive(Debug)]
pub enum HeaderError {
    Size,
    Magic,
    Version,
    Cookie,
    CachelineSize,
    AtomicSize,
}

impl From<MemError> for ChannelError {
    fn from(e: MemError) -> ChannelError {
        ChannelError::Mem(e)
    }
}

impl From<MemError> for HeaderError {
    fn from(_: MemError) -> HeaderError {
        HeaderError::Size
    }
}

impl From<MemError> for CreateError {
    fn from(e: MemError) -> CreateError {
        CreateError::Mem(e)
    }
}

impl From<Errno> for CreateError {
    fn from(errno: Errno) -> CreateError {
        CreateError::Errno(errno)
    }
}

impl From<HeaderError> for CreateError {
    fn from(field: HeaderError) -> CreateError {
        CreateError::Header(field)
    }
}
