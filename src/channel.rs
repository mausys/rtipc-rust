use std::{marker::PhantomData, mem::size_of, num::NonZeroUsize, os::fd::OwnedFd};

use  std::os::unix::io::RawFd;

use crate::{
    calc_shm_size,
    ChannelParam,
    fd::{eventfd, check_eventfd},
    error::*,
    queue::{ConsumeResult, ConsumerQueue, ProduceForceResult, ProduceTryResult, ProducerQueue},
    shm::{SharedMemory, Chunk},
    protocol::{create_request_message, parse_request_message},
    request::Request,
};

pub(crate) struct ProducerChannel {
    queue: ProducerQueue,
    info: Vec<u8>,
    eventfd: Option<OwnedFd>,
}

impl ProducerChannel {
    pub(crate) fn new(
        param: &ChannelParam,
        chunk: Chunk,
        eventfd: Option<OwnedFd>,
    ) -> Result<Self, ShmError> {
        let queue = ProducerQueue::new(chunk, param.add_msgs, param.msg_size)?;

        Ok(Self {
            queue,
            info: param.info.clone(),
            eventfd,
        })
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
    eventfd: Option<OwnedFd>,
}

impl ConsumerChannel {
    pub(crate) fn new(
        param: &ChannelParam,
        chunk: Chunk,
        eventfd: Option<OwnedFd>,
    ) -> Result<Self, ShmError> {
        let queue = ConsumerQueue::new(chunk, param.add_msgs, param.msg_size)?;

        Ok(Self {
            queue,
            info: param.info.clone(),
            eventfd,
        })
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
    info: Vec<u8>,
    eventfd: Option<OwnedFd>,
    _type: PhantomData<T>,
}

impl<T> Producer<T> {
    pub fn try_from(channel: ProducerChannel) -> Option<Self> {
        if size_of::<T>() > channel.msg_size().get() {
            return None;
        }

        Some(Self {
            queue: channel.queue,
            info: channel.info,
            eventfd: channel.eventfd,
            _type: PhantomData,
        })
    }

    pub fn msg(&mut self) -> &mut T {
        let ptr: *mut T = self.queue.current().cast();
        unsafe { &mut *ptr }
    }

    pub fn force_push(&mut self) -> ProduceForceResult {
        self.queue.force_push()
    }

    pub fn try_push(&mut self) -> ProduceTryResult {
        self.queue.try_push()
    }
}

pub struct Consumer<T> {
    queue: ConsumerQueue,
    info: Vec<u8>,
    eventfd: Option<OwnedFd>,
    _type: PhantomData<T>,
}

impl<T> Consumer<T> {
    pub(crate) fn try_from(channel: ConsumerChannel) -> Option<Self> {
        if size_of::<T>() > channel.msg_size().get() {
            return None;
        }

        Some(Self {
            queue: channel.queue,
            info: channel.info,
            eventfd: channel.eventfd,
            _type: PhantomData,
        })
    }

    pub fn msg(&self) -> Option<&T> {
        let ptr: *const T = self.queue.current()?.cast();
        Some(unsafe { &*ptr })
    }

    pub fn pop(&mut self) -> ConsumeResult {
        self.queue.pop()
    }

    pub fn flush(&mut self) -> ConsumeResult {
        self.queue.flush()
    }
}


pub struct ChannelVector {
    producers: Vec<Option<ProducerChannel>>,
    consumers: Vec<Option<ConsumerChannel>>,
    info: Vec<u8>,
}

impl ChannelVector {
    pub fn connect(
        socket: RawFd,
        producer_params: &Vec<ChannelParam>,
        consumer_params: &Vec<ChannelParam>,
        info: Vec<u8>,
    ) -> Result<Self, RtipcError> {
        let mut producers = Vec::<Option<ProducerChannel>>::with_capacity(producer_params.len());
        let mut consumers = Vec::<Option<ConsumerChannel>>::with_capacity(consumer_params.len());

        let shm_size = NonZeroUsize::new(calc_shm_size(producer_params, consumer_params))
            .ok_or(RtipcError::Argument)?;

        let msg = create_request_message(producer_params, consumer_params, &info);

        let shm = SharedMemory::new(shm_size)?;
        let fds = Vec::<RawFd>::with_capacity(10);

        let mut shm_offset = 0;

        for param in producer_params {
            let eventfd = if param.eventfd {
                Some(OwnedFd::from(eventfd()?))
            } else {
                None
            };

            let shm_size = param.shm_size();

            let chunk = shm.alloc(shm_offset, shm_size)?;
            let channel = ProducerChannel::new(param, chunk, eventfd)?;

            producers.push(Some(channel));

            shm_offset += shm_size.get();
        }

        for param in consumer_params {
            let eventfd = if param.eventfd {
                Some(OwnedFd::from(eventfd()?))
            } else {
                None
            };
            let shm_size = param.shm_size();

            let chunk = shm.alloc(shm_offset, shm_size)?;
            let channel = ConsumerChannel::new(param, chunk, eventfd)?;

            consumers.push(Some(channel));

            shm_offset += shm_size.get();
        }

        Request::new(msg, fds).send(socket)?;

        Ok(Self {
            producers,
            consumers,
            info,
        })
    }

    pub fn accept(socket: RawFd) -> Result<Self, RtipcError> {
        let mut req = Request::receive(socket)?;
        let (producer_params, consumer_params, info) = parse_request_message(req.msg())?;

        let shm_fd = req.take_fd(0).ok_or(RtipcError::Argument)?;

        let mut consumers = Vec::<Option<ConsumerChannel>>::with_capacity(consumer_params.len());
        let mut producers = Vec::<Option<ProducerChannel>>::with_capacity(producer_params.len());

        let shm = SharedMemory::from_fd(OwnedFd::from(shm_fd))?;

        let mut shm_offset = 0;
        let mut fd_index = 1;
        for param in consumer_params {
            let shm_size = param.shm_size();

            let eventfd = if param.eventfd == true {
                let fd = req.take_fd(fd_index).ok_or(RtipcError::Message(MessageError::Size))?;

                check_eventfd(fd)?;

                fd_index = fd_index + 1;
                Some(fd)
            } else {
                None
            };

            let chunk = shm.alloc(shm_offset, shm_size)?;
            let channel = ConsumerChannel::new(&param, chunk, eventfd)?;


            consumers.push(Some(channel));

            shm_offset += shm_size.get();
        }

        for param in producer_params {
            let shm_size = param.shm_size();

            let eventfd = if param.eventfd == true {
                let fd = req.take_fd(fd_index).ok_or(RtipcError::Message(MessageError::Size))?;

                check_eventfd(fd)?;

                fd_index = fd_index + 1;
                Some(fd)
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
            info,
        })
    }

    pub fn take_consumer<T>(&mut self, index: usize) -> Option<Consumer<T>> {
        let channel = self.consumers.get_mut(index)?.take()?;
        Consumer::try_from(channel)
    }

    pub fn take_producer<T>(&mut self, index: usize)  -> Option<Producer<T>>{
        let channel = self.producers.get_mut(index)?.take()?;
        Producer::try_from(channel)
    }

    pub fn info(&self) -> &Vec<u8> {
        &self.info
    }

}
