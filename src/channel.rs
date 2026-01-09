use std::{
    borrow::BorrowMut,
    marker::PhantomData,
    mem::size_of,
    num::NonZeroUsize,
    os::fd::{AsFd, BorrowedFd},
    sync::Arc,
};

use nix::sys::eventfd::EventFd;

use crate::{
    calc_shm_size,
    error::*,
    fd::{eventfd, into_eventfd},
    queue::{ConsumeResult, ConsumerQueue, ProduceForceResult, ProduceTryResult, ProducerQueue},
    shm::{Chunk, SharedMemory},
    ChannelOut, QueueConfig, VectorConfig, VectorIn, VectorOut,
};

pub(crate) struct ProducerChannel {
    queue: ProducerQueue,
    info: Vec<u8>,
    eventfd: Option<EventFd>,
}

impl ProducerChannel {
    pub(crate) fn new(
        config: &QueueConfig,
        chunk: Chunk,
        eventfd: Option<EventFd>,
    ) -> Result<Self, ShmPointerError> {
        let queue = ProducerQueue::new(chunk, config)?;

        Ok(Self {
            queue,
            info: config.info.clone(),
            eventfd,
        })
    }

    pub(crate) fn init(&self) {
        self.queue.init();
    }

    pub(crate) fn message_size(&self) -> NonZeroUsize {
        self.queue.message_size()
    }

    pub fn eventfd(&self) -> Option<BorrowedFd<'_>> {
        self.eventfd.as_ref().map(|e| e.as_fd())
    }

    pub fn info(&self) -> &Vec<u8> {
        &self.info
    }

    pub fn channel_out(&self) -> ChannelOut {
        ChannelOut {
            queue: QueueConfig {
                additional_messages: self.queue.additional_messages(),
                message_size: self.queue.message_size(),
                info: self.info.clone(),
            },
            eventfd: self.eventfd.as_ref().map(|e| e.as_fd()),
        }
    }
}

pub(crate) struct ConsumerChannel {
    queue: ConsumerQueue,
    info: Vec<u8>,
    eventfd: Option<EventFd>,
}

impl ConsumerChannel {
    pub(crate) fn new(
        config: &QueueConfig,
        chunk: Chunk,
        eventfd: Option<EventFd>,
    ) -> Result<Self, ShmPointerError> {
        let queue = ConsumerQueue::new(chunk, config)?;

        Ok(Self {
            queue,
            info: config.info.clone(),
            eventfd,
        })
    }

    pub(crate) fn init(&self) {
        self.queue.init();
    }

    pub(crate) fn message_size(&self) -> NonZeroUsize {
        self.queue.message_size()
    }

    pub fn info(&self) -> &Vec<u8> {
        &self.info
    }

    pub fn eventfd(&self) -> Option<BorrowedFd<'_>> {
        self.eventfd.as_ref().map(|e| e.as_fd())
    }

    pub fn channel_out(&self) -> ChannelOut {
        ChannelOut {
            queue: QueueConfig {
                additional_messages: self.queue.additional_messages(),
                message_size: self.queue.message_size(),
                info: self.info.clone(),
            },
            eventfd: self.eventfd.as_ref().map(|e| e.as_fd()),
        }
    }
}

pub struct Producer<T: Copy> {
    queue: ProducerQueue,
    eventfd: Option<EventFd>,
    cache: Option<Box<T>>,
}

impl<T: Copy> Producer<T> {
    fn try_from(channel: ProducerChannel) -> Option<Self> {
        if size_of::<T>() > channel.message_size().get() {
            return None;
        }

        Some(Self {
            queue: channel.queue,
            eventfd: channel.eventfd,
            cache: None,
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
    fn try_from(channel: ConsumerChannel) -> Option<Self> {
        if size_of::<T>() > channel.message_size().get() {
            return None;
        }

        Some(Self {
            queue: channel.queue,
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

pub struct ChannelVector {
    producers: Vec<Option<ProducerChannel>>,
    consumers: Vec<Option<ConsumerChannel>>,
    info: Vec<u8>,
    shm: Arc<SharedMemory>,
}

impl ChannelVector {
    pub(crate) fn new(vconfig: &VectorConfig) -> Result<Self, CreateRequestError> {
        let mut producers = Vec::<Option<ProducerChannel>>::with_capacity(vconfig.producers.len());
        let mut consumers = Vec::<Option<ConsumerChannel>>::with_capacity(vconfig.consumers.len());

        let shm_size = NonZeroUsize::new(calc_shm_size(&vconfig.producers, &vconfig.consumers))
            .ok_or(CreateRequestError::InvalidConfig)?;

        let shm = SharedMemory::new(shm_size)?;

        let mut shm_offset = 0;

        for config in &vconfig.producers {
            let eventfd = if config.eventfd {
                let efd = eventfd()?;
                Some(efd)
            } else {
                None
            };

            let shm_size = config.queue.shm_size();

            let chunk = shm.alloc(shm_offset, shm_size)?;
            let channel = ProducerChannel::new(&config.queue, chunk, eventfd)?;
            channel.init();

            producers.push(Some(channel));

            shm_offset += shm_size.get();
        }

        for config in &vconfig.consumers {
            let eventfd = if config.eventfd {
                let efd = eventfd()?;
                Some(efd)
            } else {
                None
            };
            let shm_size = config.queue.shm_size();

            let chunk = shm.alloc(shm_offset, shm_size)?;
            let channel = ConsumerChannel::new(&config.queue, chunk, eventfd)?;
            channel.init();

            consumers.push(Some(channel));

            shm_offset += shm_size.get();
        }

        Ok(Self {
            producers,
            consumers,
            info: vconfig.info.clone(),
            shm,
        })
    }

    pub(crate) fn map(vin: VectorIn) -> Result<Self, ProcessRequestError> {
        let shm = SharedMemory::from_fd(vin.shmfd)?;

        let mut consumers = Vec::<Option<ConsumerChannel>>::with_capacity(vin.consumers.len());
        let mut producers = Vec::<Option<ProducerChannel>>::with_capacity(vin.producers.len());

        let mut shm_offset = 0;

        for cin in vin.consumers {
            let shm_size = cin.queue.shm_size();

            let eventfd = cin.eventfd.map(into_eventfd).transpose()?;

            let chunk = shm.alloc(shm_offset, shm_size)?;
            let channel = ConsumerChannel::new(&cin.queue, chunk, eventfd)?;

            consumers.push(Some(channel));

            shm_offset += shm_size.get();
        }

        for cin in vin.producers {
            let shm_size = cin.queue.shm_size();

            let eventfd = cin.eventfd.map(into_eventfd).transpose()?;

            let chunk = shm.alloc(shm_offset, shm_size)?;
            let channel = ProducerChannel::new(&cin.queue, chunk, eventfd)?;

            producers.push(Some(channel));

            shm_offset += shm_size.get();
        }

        Ok(Self {
            producers,
            consumers,
            info: vin.info,
            shm,
        })
    }

    pub fn consumer_info(&self, index: usize) -> Option<&Vec<u8>> {
        self.consumers.get(index)?.as_ref().map(|c| c.info())
    }

    pub fn producer_info(&self, index: usize) -> Option<&Vec<u8>> {
        self.producers.get(index)?.as_ref().map(|c| c.info())
    }

    pub fn take_consumer<T: Copy>(&mut self, index: usize) -> Option<Consumer<T>> {
        let channel = self.consumers.get_mut(index)?.take()?;
        Consumer::try_from(channel)
    }

    pub fn take_producer<T: Copy>(&mut self, index: usize) -> Option<Producer<T>> {
        let channel = self.producers.get_mut(index)?.take()?;
        Producer::try_from(channel)
    }

    pub fn info(&self) -> &Vec<u8> {
        &self.info
    }

    pub fn collect_fds(&self) -> Vec<BorrowedFd<'_>> {
        let mut fds = Vec::<BorrowedFd<'_>>::new();

        let shmfd = self.shm.fd();

        fds.push(shmfd);

        let mut consumers: Vec<BorrowedFd<'_>> = self
            .consumers
            .iter()
            .filter_map(|e| e.as_ref().map(|c| c.eventfd()))
            .flatten()
            .collect();
        fds.append(&mut consumers);

        let mut producers: Vec<BorrowedFd<'_>> = self
            .producers
            .iter()
            .filter_map(|e| e.as_ref().map(|c| c.eventfd()))
            .flatten()
            .collect();
        fds.append(&mut producers);

        fds
    }

    pub fn vector_out(&self) -> VectorOut {
        let consumers: Vec<ChannelOut<'_>> = self
            .consumers
            .iter()
            .filter_map(|o| o.as_ref().map(|c| c.channel_out()))
            .collect();
        let producers: Vec<ChannelOut<'_>> = self
            .producers
            .iter()
            .filter_map(|o| o.as_ref().map(|c| c.channel_out()))
            .collect();
        VectorOut {
            consumers,
            producers,
            shmfd: self.shm.fd(),
            info: &self.info,
        }
    }
}
