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
    shm::{Chunk, SharedMemory, Span},
    header::Header,
    table::ChannelTable,
    channel::{ConsumerChannel, FetchResult, ProducerChannel},
};

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
    channel: ProducerChannel,
    _type: PhantomData<T>,
}

impl<T> Producer<T> {
    pub(crate) fn new(channel: ProducerChannel) -> Result<Producer<T>, MemError> {
        if size_of::<T>() > channel.msg_size().get() {
            return Err(MemError::Size);
        }
        Ok(Producer {
            channel,
            _type: PhantomData,
        })
    }

    pub fn msg(&mut self) -> &mut T {
        let ptr: *mut T = self.channel.current().cast();
        unsafe { &mut *ptr }
    }

    pub fn force_put(&mut self) -> bool {
        self.channel.force_put()
    }

    pub fn try_put(&mut self) -> bool {
        self.channel.try_put()
    }
}

pub struct Consumer<T> {
    channel: ConsumerChannel,
    _type: PhantomData<T>,
}

impl<T> Consumer<T> {
    pub(crate) fn new(channel: ConsumerChannel) -> Result<Consumer<T>, MemError> {
        if size_of::<T>() > channel.msg_size().get() {
            return Err(MemError::Size);
        }
        Ok(Consumer {
            channel,
            _type: PhantomData,
        })
    }

    pub fn msg(&self) -> Option<&T> {
        let ptr: *const T = self.channel.current()?.cast();
        Some(unsafe { &*ptr })
    }

    pub fn fetch_tail(&mut self) -> Option<&T> {
        let res = self.channel.fetch_tail();

        match res {
            FetchResult::None => None,
            FetchResult::Same => None,
            FetchResult::New => self.msg(),
        }
    }

    pub fn fetch_head(&mut self) -> Option<&T> {
        let success = self.channel.fetch_head();

        if !success {
            return None;
        }

        self.msg()
    }
}

pub struct RtIpc {
    shm: Arc<SharedMemory>,
    producers: Vec<Option<ProducerChannel>>,
    consumers: Vec<Option<ConsumerChannel>>,
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
        let mut consumers: Vec<Option<ConsumerChannel>> = Vec::with_capacity(table.consumers.len());
        let mut producers: Vec<Option<ProducerChannel>> = Vec::with_capacity(table.producers.len());

        for entry in table.consumers {
            let chunk = shm.alloc(&entry.span)?;
            let channel = ConsumerChannel::new(chunk, &entry.param)?;
            if init {
                channel.init();
            }
            consumers.push(Some(channel));
        }

        for entry in table.producers {
            let chunk = shm.alloc(&entry.span)?;
            let channel = ProducerChannel::new(chunk, &entry.param)?;
            if init {
                channel.init();
            }
            producers.push(Some(channel));
        }

        Ok(RtIpc {
            shm,
            consumers,
            producers,
        })
    }

    fn from_shm(shm: Arc<SharedMemory>, cookie: u32) -> Result<RtIpc, CreateError> {
        let chunk_header = RtIpc::chunk_header(&shm)?;
        let header = Header::from_chunk(&chunk_header, cookie)?;

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
        cookie: u32,
    ) -> Result<RtIpc, CreateError> {
        let header = Header::new(
            param_consumers.len() as u32,
            param_producers.len() as u32,
            cookie,
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
        cookie: u32,
    ) -> Result<RtIpc, CreateError> {
        let shm_size = RtIpc::calc_shm_size(param_consumers, param_producers)?;

        let shm = SharedMemory::new_anon(shm_size)?;
        RtIpc::new(shm, param_consumers, param_producers, cookie)
    }

    pub fn new_named_shm(
        param_consumers: &[ChannelParam],
        param_producers: &[ChannelParam],
        cookie: u32,
        path: &Path,
        mode: Mode,
    ) -> Result<RtIpc, CreateError> {
        let shm_size = RtIpc::calc_shm_size(param_consumers, param_producers)?;

        let shm = SharedMemory::new_named(shm_size, path, mode)?;
        RtIpc::new(shm, param_consumers, param_producers, cookie)
    }

    pub fn from_fd(fd: OwnedFd, cookie: u32) -> Result<RtIpc, CreateError> {
        let shm = SharedMemory::from_fd(fd)?;
        RtIpc::from_shm(shm, cookie)
    }

    pub fn take_consumer<T>(&mut self, index: usize) -> Result<Consumer<T>, ChannelError> {
        let channel_option: &mut Option<ConsumerChannel> =
            self.consumers.get_mut(index).ok_or(MemError::Index)?;
        let channel: ConsumerChannel = channel_option.take().ok_or(ChannelError::Index)?;
        Ok(Consumer::<T>::new(channel)?)
    }

    pub fn take_producer<T>(&mut self, index: usize) -> Result<Producer<T>, ChannelError> {
        let channel_option: &mut Option<ProducerChannel> =
            self.producers.get_mut(index).ok_or(MemError::Index)?;
        let channel: ProducerChannel = channel_option.take().ok_or(ChannelError::Index)?;
        Ok(Producer::<T>::new(channel)?)
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
