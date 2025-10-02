use std::{
    marker::PhantomData,
    mem::size_of,
    num::NonZeroUsize,
    os::fd::{AsRawFd, RawFd},
};

use nix::sys::eventfd::EventFd;

use crate::{
    calc_shm_size,
    error::*,
    fd::{eventfd, into_eventfd},
    protocol::{create_request_message, parse_request_message},
    queue::{ConsumeResult, ConsumerQueue, ProduceForceResult, ProduceTryResult, ProducerQueue},
    request::Request,
    shm::{Chunk, SharedMemory},
    ChannelParam, VectorParam,
};

pub(crate) struct ProducerChannel {
    queue: ProducerQueue,
    info: Vec<u8>,
    eventfd: Option<EventFd>,
}

impl ProducerChannel {
    pub(crate) fn new(
        param: &ChannelParam,
        chunk: Chunk,
        eventfd: Option<EventFd>,
    ) -> Result<Self, ShmError> {
        let queue = ProducerQueue::new(chunk, param.add_msgs, param.msg_size)?;

        Ok(Self {
            queue,
            info: param.info.clone(),
            eventfd,
        })
    }

    pub(crate) fn init(&self) {
        self.queue.init();
    }

    pub(crate) fn msg_size(&self) -> NonZeroUsize {
        self.queue.msg_size()
    }

    pub(crate) fn info(&self) -> &Vec<u8> {
        &self.info
    }
}

pub(crate) struct ConsumerChannel {
    queue: ConsumerQueue,
    info: Vec<u8>,
    eventfd: Option<EventFd>,
}

impl ConsumerChannel {
    pub(crate) fn new(
        param: &ChannelParam,
        chunk: Chunk,
        eventfd: Option<EventFd>,
    ) -> Result<Self, ShmError> {
        let queue = ConsumerQueue::new(chunk, param.add_msgs, param.msg_size)?;

        Ok(Self {
            queue,
            info: param.info.clone(),
            eventfd,
        })
    }

    pub(crate) fn init(&self) {
        self.queue.init();
    }

    pub(crate) fn msg_size(&self) -> NonZeroUsize {
        self.queue.msg_size()
    }

    pub fn info(&self) -> &Vec<u8> {
        &self.info
    }
}

pub struct Producer<T> {
    queue: ProducerQueue,
    eventfd: Option<EventFd>,
    _type: PhantomData<T>,
}

impl<T> Producer<T> {
    fn try_from(channel: ProducerChannel) -> Option<Self> {
        if size_of::<T>() > channel.msg_size().get() {
            return None;
        }

        Some(Self {
            queue: channel.queue,
            eventfd: channel.eventfd,
            _type: PhantomData,
        })
    }

    pub fn msg(&mut self) -> &mut T {
        let ptr: *mut T = self.queue.current().cast();
        unsafe { &mut *ptr }
    }

    pub fn force_push(&mut self) -> ProduceForceResult {
        let result = self.queue.force_push();

        if result == ProduceForceResult::Success {
            self.eventfd.as_ref().map(|ref fd| fd.write(1));
        }

        result
    }

    pub fn try_push(&mut self) -> ProduceTryResult {
        let result = self.queue.try_push();
        if result == ProduceTryResult::Success {
            self.eventfd.as_ref().map(|ref fd| fd.write(1));
        }
        result
    }
}

pub struct Consumer<T> {
    queue: ConsumerQueue,
    eventfd: Option<EventFd>,
    _type: PhantomData<T>,
}

impl<T> Consumer<T> {
    fn try_from(channel: ConsumerChannel) -> Option<Self> {
        if size_of::<T>() > channel.msg_size().get() {
            return None;
        }

        Some(Self {
            queue: channel.queue,
            eventfd: channel.eventfd,
            _type: PhantomData,
        })
    }

    pub fn msg(&self) -> Option<&T> {
        let ptr: *const T = self.queue.current()?.cast();
        Some(unsafe { &*ptr })
    }

    pub fn pop(&mut self) -> ConsumeResult {
        if let Some(eventfd) = self.eventfd.as_ref() {
            match eventfd.read() {
                Err(_) => return ConsumeResult::NoMsgAvailable,
                Ok(_) => {}
            }
        }

        self.queue.pop()
    }

    pub fn flush(&mut self) -> ConsumeResult {
        if self.eventfd.is_some() {
            let mut result = ConsumeResult::NoMsgAvailable;
            while self.pop() == ConsumeResult::Success {
                result = ConsumeResult::Success;
            }
            result
        } else {
            self.queue.flush()
        }
    }
}

pub struct ChannelVector {
    producers: Vec<Option<ProducerChannel>>,
    consumers: Vec<Option<ConsumerChannel>>,
    info: Vec<u8>,
}

impl ChannelVector {
    pub(crate) fn new(vparam: &VectorParam) -> Result<(Self, Request), RtIpcError> {
        let mut producers = Vec::<Option<ProducerChannel>>::with_capacity(vparam.producers.len());
        let mut consumers = Vec::<Option<ConsumerChannel>>::with_capacity(vparam.consumers.len());
        let mut fds = Vec::<RawFd>::new();

        let shm_size = NonZeroUsize::new(calc_shm_size(&vparam.producers, &vparam.consumers))
            .ok_or(RtIpcError::Argument)?;

        let shm = SharedMemory::new(shm_size)?;
        fds.push(shm.as_raw_fd());

        let mut shm_offset = 0;

        for param in &vparam.producers {
            let eventfd = if param.eventfd {
                let efd = eventfd()?;
                fds.push(efd.as_raw_fd());
                Some(efd)
            } else {
                None
            };

            let shm_size = param.shm_size();

            let chunk = shm.alloc(shm_offset, shm_size)?;
            let channel = ProducerChannel::new(&param, chunk, eventfd)?;
            channel.init();

            producers.push(Some(channel));

            shm_offset += shm_size.get();
        }

        for param in &vparam.consumers {
            let eventfd = if param.eventfd {
                let efd = eventfd()?;
                fds.push(efd.as_raw_fd());
                Some(efd)
            } else {
                None
            };
            let shm_size = param.shm_size();

            let chunk = shm.alloc(shm_offset, shm_size)?;
            let channel = ConsumerChannel::new(&param, chunk, eventfd)?;
            channel.init();

            consumers.push(Some(channel));

            shm_offset += shm_size.get();
        }

        let msg = create_request_message(&vparam);

        Ok((
            Self {
                producers,
                consumers,
                info: vparam.info.clone(),
            },
            Request::new(msg, fds),
        ))
    }

    pub(crate) fn from_request(mut req: Request) -> Result<Self, RtIpcError> {
        let vparam = parse_request_message(req.msg())?;

        let shm_fd = req.take_fd(0).ok_or(RtIpcError::Argument)?;

        let mut consumers = Vec::<Option<ConsumerChannel>>::with_capacity(vparam.consumers.len());
        let mut producers = Vec::<Option<ProducerChannel>>::with_capacity(vparam.producers.len());

        let shm = SharedMemory::from_fd(shm_fd)?;

        let mut shm_offset = 0;
        let mut fd_index = 1;
        for param in vparam.consumers {
            let shm_size = param.shm_size();

            let eventfd = if param.eventfd {
                let ofd = req
                    .take_fd(fd_index)
                    .ok_or(RtIpcError::Message(MessageError::Size))?;

                let efd = into_eventfd(ofd)?;

                fd_index += 1;
                Some(efd)
            } else {
                None
            };

            let chunk = shm.alloc(shm_offset, shm_size)?;
            let channel = ConsumerChannel::new(&param, chunk, eventfd)?;

            consumers.push(Some(channel));

            shm_offset += shm_size.get();
        }

        for param in vparam.producers {
            let shm_size = param.shm_size();

            let eventfd = if param.eventfd {
                let ofd = req
                    .take_fd(fd_index)
                    .ok_or(RtIpcError::Message(MessageError::Size))?;

                let efd = into_eventfd(ofd)?;

                fd_index += 1;
                Some(efd)
            } else {
                None
            };

            let chunk = shm.alloc(shm_offset, shm_size)?;
            let channel = ProducerChannel::new(&param, chunk, eventfd)?;

            producers.push(Some(channel));

            shm_offset += shm_size.get();
        }

        Ok(Self {
            producers,
            consumers,
            info: vparam.info.clone(),
        })
    }

    pub fn consumer_info(&self, index: usize) -> Option<&Vec<u8>> {
        self.consumers.get(index)?.as_ref().map(|c| c.info())
    }

    pub fn producer_info(&self, index: usize) -> Option<&Vec<u8>> {
        self.producers.get(index)?.as_ref().map(|c| c.info())
    }

    pub fn take_consumer<T>(&mut self, index: usize) -> Option<Consumer<T>> {
        let channel = self.consumers.get_mut(index)?.take()?;
        Consumer::try_from(channel)
    }

    pub fn take_producer<T>(&mut self, index: usize) -> Option<Producer<T>> {
        let channel = self.producers.get_mut(index)?.take()?;
        Producer::try_from(channel)
    }

    pub fn info(&self) -> &Vec<u8> {
        &self.info
    }
}
