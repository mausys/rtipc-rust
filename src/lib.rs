#[cfg(feature = "predefined_cacheline_size")]
mod cache_env;
#[cfg(not(feature = "predefined_cacheline_size"))]
mod cache_linux;
mod channel;
pub mod error;
mod fd;
mod header;
mod protocol;
mod queue;
mod shm;
mod socket;
mod unix_message;

#[macro_use]
extern crate nix;

use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::{num::NonZeroUsize, sync::atomic::AtomicU32};

#[cfg(feature = "predefined_cacheline_size")]
use crate::cache_env::max_cacheline_size;
#[cfg(not(feature = "predefined_cacheline_size"))]
use crate::cache_linux::max_cacheline_size;

pub use channel::{ChannelVector, Consumer, Producer};
pub use error::*;
pub use queue::{ConsumeResult, ProduceForceResult, ProduceTryResult};
pub use socket::{client_connect, client_connect_fd, Server};

pub use log;

pub(crate) type AtomicIndex = AtomicU32;
pub(crate) type Index = u32;
pub(crate) const MIN_MSGS: usize = 3;

pub(crate) fn mem_align(size: usize, alignment: usize) -> usize {
    (size + alignment - 1) & !(alignment - 1)
}

pub(crate) fn cacheline_aligned(size: usize) -> usize {
    mem_align(size, max_cacheline_size())
}

#[derive(Clone)]
pub struct QueueConfig {
    pub additional_messages: usize,
    pub message_size: NonZeroUsize,
    pub info: Vec<u8>,
}

#[derive(Clone)]
pub struct ChannelConfig {
    pub queue: QueueConfig,
    pub eventfd: bool,
}

impl QueueConfig {
    fn data_size(&self) -> usize {
        let n = MIN_MSGS + self.additional_messages;

        n * cacheline_aligned(self.message_size.get())
    }

    fn queue_size(&self) -> usize {
        let n = 2 + MIN_MSGS + self.additional_messages;
        cacheline_aligned(n * std::mem::size_of::<Index>())
    }

    pub(crate) fn shm_size(&self) -> NonZeroUsize {
        NonZeroUsize::new(self.queue_size() + self.data_size()).unwrap()
    }
}

pub struct VectorConfig {
    pub producers: Vec<ChannelConfig>,
    pub consumers: Vec<ChannelConfig>,
    pub info: Vec<u8>,
}

pub struct ChannelIn {
    pub queue: QueueConfig,
    pub eventfd: Option<OwnedFd>,
}

impl ChannelIn {
    fn new(config: &QueueConfig, eventfd: Option<OwnedFd>) -> Self {
        ChannelIn {
            queue: config.clone(),
            eventfd,
        }
    }
}

pub struct VectorIn {
    pub producers: Vec<ChannelIn>,
    pub consumers: Vec<ChannelIn>,
    pub info: Vec<u8>,
    pub shmfd: OwnedFd,
}

pub struct ChannelOut<'a> {
    pub queue: QueueConfig,
    pub eventfd: Option<BorrowedFd<'a>>,
}

impl<'a> ChannelOut<'a> {
    fn new(config: &QueueConfig, eventfd: Option<BorrowedFd<'a>>) -> Self {
        ChannelOut {
            queue: config.clone(),
            eventfd: eventfd,
        }
    }
}

pub struct VectorOut<'a> {
    pub producers: Vec<ChannelOut<'a>>,
    pub consumers: Vec<ChannelOut<'a>>,
    pub info: &'a Vec<u8>,
    pub shmfd: BorrowedFd<'a>,
}

pub(crate) fn calc_shm_size(group0: &[ChannelConfig], group1: &[ChannelConfig]) -> usize {
    let mut size = 0;

    for config in group0 {
        size += config.queue.shm_size().get();
    }

    for config in group1 {
        size += config.queue.shm_size().get();
    }

    size
}
