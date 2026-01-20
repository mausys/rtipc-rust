use std::{
    collections::VecDeque,
    num::NonZeroUsize,
    os::fd::{AsFd, BorrowedFd, OwnedFd},
};

use nix::sys::eventfd::EventFd;

use crate::{
    ChannelConfig, QueueConfig, VectorConfig, calc_shm_size,
    error::*,
    unix::{check_memfd, eventfd_create, into_eventfd, shmfd_create},
};
use nix::errno::Errno;

pub struct ChannelResource {
    pub config: QueueConfig,
    pub eventfd: Option<EventFd>,
}

impl ChannelResource {
    pub fn new(config: &QueueConfig, eventfd_raw: Option<OwnedFd>) -> Result<Self, Errno> {
        let eventfd = eventfd_raw.map(into_eventfd).transpose()?;
        Ok(Self {
            config: config.clone(),
            eventfd,
        })
    }
}

pub struct VectorResource {
    pub consumers: Vec<ChannelResource>,
    pub producers: Vec<ChannelResource>,
    pub info: Vec<u8>,
    pub shmfd: OwnedFd,
    pub owner: bool,
}

impl VectorResource {
    fn create_channel_resources(
        configs: &Vec<ChannelConfig>,
        mut eventfds: VecDeque<OwnedFd>,
    ) -> Result<Vec<ChannelResource>, TransferError> {
        let mut channels = Vec::<ChannelResource>::with_capacity(configs.len());

        for config in configs {
            let eventfd = if config.eventfd {
                let eventfd = eventfds
                    .pop_front()
                    .ok_or(TransferError::MissingFileDescriptor)?;
                Some(eventfd)
            } else {
                None
            };

            let channel = ChannelResource::new(&config.queue, eventfd)?;

            channels.push(channel);
        }

        Ok(channels)
    }
    pub fn new(
        vconfig: &VectorConfig,
        shmfd: OwnedFd,
        consumer_eventfds: VecDeque<OwnedFd>,
        producer_eventfds: VecDeque<OwnedFd>,
    ) -> Result<Self, TransferError> {
        check_memfd(shmfd.as_fd())?;

        let consumers = Self::create_channel_resources(&vconfig.consumers, consumer_eventfds)?;
        let producers = Self::create_channel_resources(&vconfig.producers, producer_eventfds)?;

        Ok(Self {
            producers,
            consumers,
            info: vconfig.info.clone(),
            shmfd,
            owner: false,
        })
    }

    pub fn allocate(vconfig: &VectorConfig) -> Result<Self, ResourceError> {
        let mut producers = Vec::<ChannelResource>::with_capacity(vconfig.producers.len());
        let mut consumers = Vec::<ChannelResource>::with_capacity(vconfig.consumers.len());

        let shm_size = NonZeroUsize::new(calc_shm_size(&vconfig.producers, &vconfig.consumers))
            .ok_or(ResourceError::InvalidArgument)?;

        let shmfd = shmfd_create(shm_size)?;

        for config in &vconfig.consumers {
            let eventfd = if config.eventfd {
                let eventfd = eventfd_create()?;
                Some(eventfd)
            } else {
                None
            };

            let channel = ChannelResource {
                config: config.queue.clone(),
                eventfd,
            };

            consumers.push(channel);
        }

        for config in &vconfig.producers {
            let eventfd = if config.eventfd {
                let eventfd = eventfd_create()?;
                Some(eventfd)
            } else {
                None
            };

            let channel = ChannelResource {
                config: config.queue.clone(),
                eventfd,
            };

            producers.push(channel);
        }

        Ok(Self {
            consumers,
            producers,
            info: vconfig.info.clone(),
            shmfd,
            owner: true,
        })
    }

    pub fn add_consumer(
        &mut self,
        config: &QueueConfig,
        eventfd: Option<OwnedFd>,
    ) -> Result<(), Errno> {
        let channel = ChannelResource::new(config, eventfd)?;
        self.consumers.push(channel);
        Ok(())
    }

    pub fn add_producer(
        &mut self,
        config: &QueueConfig,
        eventfd: Option<OwnedFd>,
    ) -> Result<(), Errno> {
        let channel = ChannelResource::new(config, eventfd)?;
        self.producers.push(channel);
        Ok(())
    }

    pub fn consumer_info(&self, index: usize) -> Option<&Vec<u8>> {
        self.consumers.get(index).map(|c| &c.config.info)
    }

    pub fn producer_info(&self, index: usize) -> Option<&Vec<u8>> {
        self.consumers.get(index).map(|c| &c.config.info)
    }

    pub fn info(&self) -> &Vec<u8> {
        &self.info
    }

    pub fn shmfd(&self) -> BorrowedFd<'_> {
        self.shmfd.as_fd()
    }

    fn collect_eventfds(channels: &[ChannelResource]) -> Vec<BorrowedFd<'_>> {
        let fds: Vec<BorrowedFd<'_>> = channels
            .iter()
            .filter_map(|c| c.eventfd.as_ref().map(|fd| fd.as_fd()))
            .collect();

        fds
    }

    pub fn collect_consumer_eventfds(&self) -> Vec<BorrowedFd<'_>> {
        Self::collect_eventfds(&self.consumers)
    }

    pub fn collect_producer_eventfds(&self) -> Vec<BorrowedFd<'_>> {
        Self::collect_eventfds(&self.producers)
    }
}
