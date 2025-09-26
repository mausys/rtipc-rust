use std::{marker::PhantomData, mem::size_of, num::NonZeroUsize, os::fd::OwnedFd};

use crate::{
    error::*,
    queue::{ConsumeResult, ConsumerQueue, ProduceForceResult, ProduceTryResult, ProducerQueue},
    shm::Chunk,
    ChannelParam,
};

pub struct ProducerChannel {
    queue: ProducerQueue,
    info: Vec<u8>,
    eventfd: Option<OwnedFd>,
}

impl ProducerChannel {
    pub(crate) fn new(
        param: &ChannelParam,
        chunk: Chunk,
        eventfd: Option<OwnedFd>,
    ) -> Result<Self, MemError> {
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
}

pub struct ConsumerChannel {
    queue: ConsumerQueue,
    info: Vec<u8>,
    eventfd: Option<OwnedFd>,
}

impl ConsumerChannel {
    pub(crate) fn new(
        param: &ChannelParam,
        chunk: Chunk,
        eventfd: Option<OwnedFd>,
    ) -> Result<Self, MemError> {
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
}

pub struct Producer<T> {
    queue: ProducerQueue,
    info: Vec<u8>,
    eventfd: Option<OwnedFd>,
    _type: PhantomData<T>,
}

impl<T> Producer<T> {
    pub(crate) fn try_from(channel: ProducerChannel) -> Result<Self, MemError> {
        if size_of::<T>() > channel.msg_size().get() {
            return Err(MemError::Size);
        }

        Ok(Self {
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
    pub(crate) fn try_from(channel: ConsumerChannel) -> Result<Self, MemError> {
        if size_of::<T>() > channel.msg_size().get() {
            return Err(MemError::Size);
        }

        Ok(Self {
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
