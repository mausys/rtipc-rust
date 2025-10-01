mod cache;
mod channel;
pub mod error;
mod fd;
mod header;
mod protocol;
mod queue;
mod request;
mod shm;
mod socket;

#[macro_use]
extern crate nix;

use std::{num::NonZeroUsize, os::fd::OwnedFd, sync::atomic::AtomicU32};

use crate::cache::cacheline_aligned;

pub use channel::{ChannelVector, Consumer, Producer};
pub use error::*;
pub use queue::{ConsumeResult, ProduceForceResult, ProduceTryResult};

pub use log;

pub(crate) type AtomicIndex = AtomicU32;
pub(crate) type Index = u32;
pub(crate) const MIN_MSGS: usize = 3;

pub(crate) fn mem_align(size: usize, alignment: usize) -> usize {
    (size + alignment - 1) & !(alignment - 1)
}

pub struct ChannelParam {
    pub add_msgs: usize,
    pub msg_size: NonZeroUsize,
    info: Vec<u8>,
    eventfd: bool,
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

    pub(crate) fn shm_size(&self) -> NonZeroUsize {
        NonZeroUsize::new(self.queue_size() + self.data_size()).unwrap()
    }
}

pub struct Server {
    socket: OwnedFd,
}

pub(crate) fn calc_shm_size(group0: &[ChannelParam], group1: &[ChannelParam]) -> usize {
    let mut size = 0;

    for param in group0 {
        size += param.shm_size().get();
    }

    for param in group1 {
        size += param.shm_size().get();
    }

    size
}
