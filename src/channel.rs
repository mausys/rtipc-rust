use std::{marker::PhantomData, mem::size_of, os::fd::OwnedFd};

use crate::{
    error::*,
    queue::{ConsumeResult, ConsumerQueue, ProduceForceResult, ProduceTryResult, ProducerQueue},
    shm::Chunk,
    ChannelParam,
};

pub struct Producer<T> {
    queue: ProducerQueue,
    info: Option<Vec<u8>>,
    eventfd: Option<OwnedFd>,
    _type: PhantomData<T>,
}

impl<T> Producer<T> {
    pub(crate) fn new(param: ChannelParam, chunk: Chunk) -> Result<Producer<T>, MemError> {
        if size_of::<T>() > param.msg_size.get() {
            return Err(MemError::Size);
        }

        let queue = ProducerQueue::new(chunk, param.add_msgs, param.msg_size)?;

        Ok(Producer {
            queue,
            info: param.info,
            eventfd: None,
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
    info: Option<Vec<u8>>,
    eventfd: Option<OwnedFd>,
    _type: PhantomData<T>,
}

impl<T> Consumer<T> {
    pub(crate) fn new(param: ChannelParam, chunk: Chunk) -> Result<Consumer<T>, MemError> {
        if size_of::<T>() > param.msg_size.get() {
            return Err(MemError::Size);
        }

        let queue = ConsumerQueue::new(chunk, param.add_msgs, param.msg_size)?;

        Ok(Consumer {
            queue,
            info: param.info,
            eventfd: None,
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
