use std::{
    borrow::BorrowMut,
    marker::PhantomData,
    mem::size_of,
    os::fd::{AsFd, BorrowedFd},
};

use nix::sys::eventfd::EventFd;

use crate::{
    error::*,
    queue::{
        ConsumeResult, ConsumerQueue, ProduceForceResult, ProduceTryResult, ProducerQueue, Queue,
    },
    resource::{ChannelResource, VectorResource},
    shm::SharedMemory,
};

pub struct Producer<T: Copy> {
    queue: ProducerQueue,
    eventfd: Option<EventFd>,
    cache: Option<Box<T>>,
    _type: PhantomData<T>,
}

impl<T: Copy> Producer<T> {
    fn new(channel: Channel) -> Result<Self, ShmMapError> {
        if size_of::<T>() > channel.queue.message_size().get() {
            return Err(ShmMapError::OutOfBounds);
        }

        let queue = ProducerQueue::new(channel.queue);

        Ok(Self {
            queue,
            eventfd: channel.eventfd,
            cache: None,
            _type: PhantomData,
        })
    }

    pub fn current_message(&mut self) -> &mut T {
        if let Some(ref mut cache) = self.cache {
            cache.borrow_mut()
        } else {
            unsafe { &mut *self.queue.current_message().cast::<T>() }
        }
    }

    pub fn force_push(&mut self) -> ProduceForceResult {
        if let Some(ref cache) = self.cache {
            *self.current_message() = *cache.clone();
        }

        let result = self.queue.force_push();

        if result == ProduceForceResult::Success {
            self.eventfd.as_ref().map(|fd| fd.write(1));
        }

        result
    }

    pub fn try_push(&mut self) -> ProduceTryResult {
        if let Some(ref cache) = self.cache {
            if self.queue.full() {
                return ProduceTryResult::QueueFull;
            }
            *self.current_message() = *cache.clone();
        }

        let result = self.queue.try_push();
        if result == ProduceTryResult::Success {
            self.eventfd.as_ref().map(|fd| fd.write(1));
        }
        result
    }

    pub fn eventfd(&self) -> Option<BorrowedFd> {
        self.eventfd.as_ref().map(|fd| fd.as_fd())
    }

    pub fn take_eventfd(&mut self) -> Option<EventFd> {
        self.eventfd.take()
    }

    pub fn enable_cache(&mut self) {
        if self.cache.is_none() {
            self.cache = Some(Box::new(*self.current_message()));
        }
    }

    pub fn disable_cache(&mut self) {
        if let Some(cache) = self.cache.take() {
            *self.current_message() = *cache;
        }
    }
}

pub struct Consumer<T: Copy> {
    queue: ConsumerQueue,
    eventfd: Option<EventFd>,
    _type: PhantomData<T>,
}

impl<T: Copy> Consumer<T> {
    fn new(channel: Channel) -> Result<Self, ShmMapError> {
        if size_of::<T>() > channel.queue.message_size().get() {
            return Err(ShmMapError::OutOfBounds);
        }

        let queue = ConsumerQueue::new(channel.queue);

        Ok(Self {
            queue,
            eventfd: channel.eventfd,
            _type: PhantomData,
        })
    }

    pub fn current_message(&self) -> Option<&T> {
        let ptr: *const T = self.queue.current_message()?.cast();
        Some(unsafe { &*ptr })
    }

    pub fn pop(&mut self) -> ConsumeResult {
        if let Some(eventfd) = self.eventfd.as_ref() {
            if eventfd.read().is_err() {
                if self.queue.current_message().is_some() {
                    return ConsumeResult::NoNewMessage;
                } else {
                    return ConsumeResult::NoMessage;
                }
            }
        }

        self.queue.pop()
    }

    pub fn flush(&mut self) -> ConsumeResult {
        if self.eventfd.is_some() {
            let mut result = ConsumeResult::NoMessage;
            while self.pop() == ConsumeResult::Success {
                result = ConsumeResult::Success;
            }
            result
        } else {
            self.queue.flush()
        }
    }

    pub fn eventfd(&self) -> Option<BorrowedFd> {
        self.eventfd.as_ref().map(|fd| fd.as_fd())
    }

    pub fn take_eventfd(&mut self) -> Option<EventFd> {
        self.eventfd.take()
    }
}

pub(crate) struct Channel {
    queue: Queue,
    eventfd: Option<EventFd>,
}

pub struct ChannelVector {
    producers: Vec<Option<Channel>>,
    consumers: Vec<Option<Channel>>,
    info: Vec<u8>,
}

impl ChannelVector {
    fn create_channels(
        rscs: Vec<ChannelResource>,
        shm: &SharedMemory,
        shm_offset: &mut usize,
        shm_init: bool,
    ) -> Result<Vec<Option<Channel>>, ShmMapError> {
        let mut channels = Vec::<Option<Channel>>::with_capacity(rscs.len());

        for rsc in rscs {
            let shm_size = rsc.config.shm_size();

            let chunk = shm.alloc(*shm_offset, shm_size)?;
            let queue = Queue::new(chunk, rsc.config)?;

            if shm_init {
                queue.init();
            }

            let channel = Channel {
                queue,
                eventfd: rsc.eventfd,
            };

            channels.push(Some(channel));

            *shm_offset += shm_size.get();
        }
        Ok(channels)
    }

    pub(crate) fn new(vrsc: VectorResource) -> Result<Self, ResourceError> {
        let shm = SharedMemory::new(vrsc.shmfd)?;

        let mut shm_offset = 0;

        let consumers;
        let producers;

        if vrsc.owner {
            producers = Self::create_channels(vrsc.producers, &shm, &mut shm_offset, !vrsc.owner)?;
            consumers = Self::create_channels(vrsc.consumers, &shm, &mut shm_offset, !vrsc.owner)?;
        } else {
            consumers = Self::create_channels(vrsc.consumers, &shm, &mut shm_offset, !vrsc.owner)?;
            producers = Self::create_channels(vrsc.producers, &shm, &mut shm_offset, !vrsc.owner)?;
        }

        Ok(Self {
            producers,
            consumers,
            info: vrsc.info,
        })
    }

    pub fn take_consumer<T: Copy>(&mut self, index: usize) -> Option<Consumer<T>> {
        let channel = self.consumers.get_mut(index)?.take()?;
        let consumer = Consumer::new(channel).ok()?;
        Some(consumer)
    }

    pub fn take_producer<T: Copy>(&mut self, index: usize) -> Option<Producer<T>> {
        let channel = self.producers.get_mut(index)?.take()?;
        let producer = Producer::new(channel).ok()?;
        Some(producer)
    }

    pub fn info(&self) -> &Vec<u8> {
        &self.info
    }
}
