use nix::errno::Errno;

#[derive(Debug)]
pub enum ShmPointerError {
    OutOfBounds,
    Misalignment,
}

#[derive(Debug)]
pub enum RequestPointerError {
    OutOfBounds,
    Misalignment,
}

#[derive(Debug)]
pub enum HeaderError {
    SizeExceedsRequest,
    MagicMismatch,
    VersionMismatch,
    CachelineSizeMismatch,
    AtomicSizeMismatch,
}

#[derive(Debug)]
pub enum ProcessRequestError {
    Errno(Errno),
    RequestPointerError(RequestPointerError),
    ShmPoniterError(ShmPointerError),
    HeaderError(HeaderError),
    MissingFileDescriptor,
    ResponseError,
}

#[derive(Debug)]
pub enum CreateRequestError {
    InvalidConfig,
    Errno(Errno),
    RequestPointerError(RequestPointerError),
    ShmPoniterError(ShmPointerError),
    HeaderError(HeaderError),
    ResponseError,
}

#[derive(Debug)]
pub enum RtIpcError {
    Errno(Errno),
    Shm(ShmPointerError),
    Message(ProcessRequestError),
    Argument,
}

impl From<Errno> for CreateRequestError {
    fn from(e: Errno) -> CreateRequestError {
        CreateRequestError::Errno(e)
    }
}

impl From<ShmPointerError> for CreateRequestError {
    fn from(e: ShmPointerError) -> CreateRequestError {
        CreateRequestError::ShmPoniterError(e)
    }
}

impl From<RequestPointerError> for CreateRequestError {
    fn from(e: RequestPointerError) -> CreateRequestError {
        CreateRequestError::RequestPointerError(e)
    }
}

impl From<HeaderError> for CreateRequestError {
    fn from(e: HeaderError) -> CreateRequestError {
        CreateRequestError::HeaderError(e)
    }
}

impl From<Errno> for ProcessRequestError {
    fn from(e: Errno) -> ProcessRequestError {
        ProcessRequestError::Errno(e)
    }
}

impl From<ShmPointerError> for ProcessRequestError {
    fn from(e: ShmPointerError) -> ProcessRequestError {
        ProcessRequestError::ShmPoniterError(e)
    }
}

impl From<RequestPointerError> for ProcessRequestError {
    fn from(e: RequestPointerError) -> ProcessRequestError {
        ProcessRequestError::RequestPointerError(e)
    }
}

impl From<HeaderError> for ProcessRequestError {
    fn from(e: HeaderError) -> ProcessRequestError {
        ProcessRequestError::HeaderError(e)
    }
}
