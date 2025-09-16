mod cache;
mod channel;
pub mod error;
mod header;
mod shm;
mod table;

use std::{
    fmt,
    marker::PhantomData,
    mem::size_of,
    num::NonZeroUsize,
    os::fd::OwnedFd,
    path::Path,
    sync::{atomic::AtomicU32, Arc},
};

use nix::sys::stat::Mode;

use crate::{
    cache::cacheline_aligned,
    channel::{ConsumerQueue, ProducerQueue},
    header::Header,
    shm::{Chunk, SharedMemory, Span},
    table::ChannelTable,
};

pub use channel::{ConsumeResult, ProduceForceResult, ProduceTryResult};
pub use error::*;

pub use log;

pub(crate) type AtomicIndex = AtomicU32;
pub(crate) type Index = u32;
pub(crate) const MIN_MSGS: usize = 3;

#[derive(Debug, Copy, Clone)]
pub struct ChannelParam {
    pub add_msgs: usize,
    pub msg_size: NonZeroUsize,
}

impl ChannelParam {
    fn data_size(&self) -> usize {
        let n = MIN_MSGS + self.add_msgs;

        n * cacheline_aligned(self.msg_size.get())
    }

    fn queue_size(&self) -> usize {
        let n = 2 + MIN_MSGS + self.add_msgs;
        cacheline_aligned(n * std::mem::size_of::<Index>())
    }

    pub(crate) fn size(&self) -> NonZeroUsize {
        NonZeroUsize::new(self.queue_size() + self.data_size()).unwrap()
    }
}

pub struct Producer<T> {
    queue: ProducerQueue,
    _type: PhantomData<T>,
}

impl<T> Producer<T> {
    pub(crate) fn new(queue: ProducerQueue) -> Result<Producer<T>, MemError> {
        if size_of::<T>() > queue.msg_size().get() {
            return Err(MemError::Size);
        }
        Ok(Producer {
            queue,
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
    _type: PhantomData<T>,
}

impl<T> Consumer<T> {
    pub(crate) fn new(queue: ConsumerQueue) -> Result<Consumer<T>, MemError> {
        if size_of::<T>() > queue.msg_size().get() {
            return Err(MemError::Size);
        }
        Ok(Consumer {
            queue,
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

pub struct RtIpc {
    shm: Arc<SharedMemory>,
    producers: Vec<Option<ProducerQueue>>,
    consumers: Vec<Option<ConsumerQueue>>,
}

impl RtIpc {
    fn calc_shm_size(
        consumers: &[ChannelParam],
        producers: &[ChannelParam],
    ) -> Result<NonZeroUsize, CreateError> {
        let num_channels =
            NonZeroUsize::new(consumers.len() + producers.len()).ok_or(CreateError::Argument)?;

        let mut size = RtIpc::calc_offset_channels(num_channels);

        for chan in consumers {
            size += chan.size().get();
        }

        for chan in producers {
            size += chan.size().get();
        }

        NonZeroUsize::new(size).ok_or(CreateError::Argument)
    }

    fn calc_offset_channels(num_channels: NonZeroUsize) -> usize {
        let mut offset = size_of::<Header>();
        offset += ChannelTable::calc_size(num_channels).get();
        offset = cacheline_aligned(offset);
        offset
    }

    fn chunk_header(shm: &SharedMemory) -> Result<Chunk, MemError> {
        let span = Span {
            offset: 0,
            size: NonZeroUsize::new(size_of::<Header>()).unwrap(),
        };
        shm.alloc(&span)
    }

    fn chunk_table(shm: &SharedMemory, num_channels: NonZeroUsize) -> Result<Chunk, MemError> {
        let offset = size_of::<Header>();
        let size = ChannelTable::calc_size(num_channels);
        let span = Span { offset, size };
        shm.alloc(&span)
    }

    fn construct(
        shm: Arc<SharedMemory>,
        table: ChannelTable,
        init: bool,
    ) -> Result<RtIpc, CreateError> {
        let mut consumers: Vec<Option<ConsumerQueue>> = Vec::with_capacity(table.consumers.len());
        let mut producers: Vec<Option<ProducerQueue>> = Vec::with_capacity(table.producers.len());

        for entry in table.consumers {
            let chunk = shm.alloc(&entry.span)?;
            let queue = ConsumerQueue::new(chunk, &entry.param)?;
            if init {
                queue.init();
            }
            consumers.push(Some(queue));
        }

        for entry in table.producers {
            let chunk = shm.alloc(&entry.span)?;
            let queue = ProducerQueue::new(chunk, &entry.param)?;
            if init {
                queue.init();
            }
            producers.push(Some(queue));
        }

        Ok(RtIpc {
            shm,
            consumers,
            producers,
        })
    }

    fn from_shm(shm: Arc<SharedMemory>) -> Result<RtIpc, CreateError> {
        let chunk_header = RtIpc::chunk_header(&shm)?;
        let header = Header::from_chunk(&chunk_header)?;

        let num_producers = header.num_channels[0] as usize;
        let num_consumers = header.num_channels[1] as usize;

        let num_channels =
            NonZeroUsize::new(num_consumers + num_producers).ok_or(CreateError::Argument)?;

        let offset: usize = RtIpc::calc_offset_channels(num_channels);

        let chunk_table = RtIpc::chunk_table(&shm, num_channels)?;

        let table = ChannelTable::from_chunk(&chunk_table, num_consumers, num_producers, offset)?;

        RtIpc::construct(shm, table, false)
    }

    fn new(
        shm: Arc<SharedMemory>,
        param_consumers: &[ChannelParam],
        param_producers: &[ChannelParam],
    ) -> Result<RtIpc, CreateError> {
        let header = Header::new(
            param_consumers.len() as u32,
            param_producers.len() as u32,
        );
        let num_channels = NonZeroUsize::new(param_consumers.len() + param_producers.len())
            .ok_or(CreateError::Argument)?;

        let offset: usize = RtIpc::calc_offset_channels(num_channels);
        let table = ChannelTable::new(param_consumers, param_producers, offset);

        let chunk_header = RtIpc::chunk_header(&shm)?;
        let chunk_table = RtIpc::chunk_table(&shm, num_channels)?;

        header.write(&chunk_header)?;
        table.write(&chunk_table)?;

        RtIpc::construct(shm, table, true)
    }

    pub fn new_anon_shm(
        param_consumers: &[ChannelParam],
        param_producers: &[ChannelParam],
    ) -> Result<RtIpc, CreateError> {
        let shm_size = RtIpc::calc_shm_size(param_consumers, param_producers)?;

        let shm = SharedMemory::new_anon(shm_size)?;
        RtIpc::new(shm, param_consumers, param_producers)
    }

    pub fn new_named_shm(
        param_consumers: &[ChannelParam],
        param_producers: &[ChannelParam],
        path: &Path,
        mode: Mode,
    ) -> Result<RtIpc, CreateError> {
        let shm_size = RtIpc::calc_shm_size(param_consumers, param_producers)?;

        let shm = SharedMemory::new_named(shm_size, path, mode)?;
        RtIpc::new(shm, param_consumers, param_producers)
    }

    pub fn from_fd(fd: OwnedFd) -> Result<RtIpc, CreateError> {
        let shm = SharedMemory::from_fd(fd)?;
        RtIpc::from_shm(shm)
    }

    pub fn take_consumer<T>(&mut self, index: usize) -> Result<Consumer<T>, ChannelError> {
        let channel_option: &mut Option<ConsumerQueue> =
            self.consumers.get_mut(index).ok_or(MemError::Index)?;
        let queue: ConsumerQueue = channel_option.take().ok_or(ChannelError::Index)?;
        Ok(Consumer::<T>::new(queue)?)
    }

    pub fn take_producer<T>(&mut self, index: usize) -> Result<Producer<T>, ChannelError> {
        let channel_option: &mut Option<ProducerQueue> =
            self.producers.get_mut(index).ok_or(MemError::Index)?;
        let queue: ProducerQueue = channel_option.take().ok_or(ChannelError::Index)?;
        Ok(Producer::<T>::new(queue)?)
    }

    pub fn get_fd(&self) -> &OwnedFd {
        self.shm.get_fd()
    }
}

impl fmt::Display for RtIpc {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "shm: {}", self.shm)
    }
}
