use std::{
    collections::VecDeque,
    num::NonZeroUsize,
    os::fd::{AsFd, BorrowedFd, OwnedFd},
};

use nix::sys::eventfd::EventFd;

use crate::{
    calc_shm_size,
    error::*,
    unix::{check_memfd, eventfd_create, into_eventfd, shmfd_create},
    QueueConfig, VectorConfig,
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
    pub producers: Vec<ChannelResource>,
    pub consumers: Vec<ChannelResource>,
    pub info: Vec<u8>,
    pub shmfd: OwnedFd,
    pub owner: bool,
}

impl VectorResource {
    pub fn new(
        vconfig: &VectorConfig,
        shmfd: OwnedFd,
        mut eventfds: VecDeque<OwnedFd>,
    ) -> Result<Self, TransferError> {
        check_memfd(shmfd.as_fd())?;

        let mut consumers = Vec::<ChannelResource>::with_capacity(vconfig.consumers.len());
        let mut producers = Vec::<ChannelResource>::with_capacity(vconfig.producers.len());

        for config in &vconfig.consumers {
            let eventfd = if config.eventfd {
                let eventfd = eventfds
                    .pop_front()
                    .ok_or(TransferError::MissingFileDescriptor)?;
                Some(eventfd)
            } else {
                None
            };

            let channel = ChannelResource::new(&config.queue, eventfd)?;

            consumers.push(channel);
        }

        for config in &vconfig.producers {
            let eventfd = if config.eventfd {
                let eventfd = eventfds
                    .pop_front()
                    .ok_or(TransferError::MissingFileDescriptor)?;
                Some(eventfd)
            } else {
                None
            };

            let channel = ChannelResource::new(&config.queue, eventfd)?;

            producers.push(channel);
        }

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

        Ok(Self {
            producers,
            consumers,
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

    pub fn collect_fds(&self) -> Vec<BorrowedFd<'_>> {
        let mut fds = Vec::<BorrowedFd<'_>>::new();

        fds.push(self.shmfd.as_fd());

        let mut producers: Vec<BorrowedFd<'_>> = self
            .producers
            .iter()
            .filter_map(|c| c.eventfd.as_ref().map(|fd| fd.as_fd()))
            .collect();
        fds.append(&mut producers);

        let mut consumers: Vec<BorrowedFd<'_>> = self
            .consumers
            .iter()
            .filter_map(|c| c.eventfd.as_ref().map(|fd| fd.as_fd()))
            .collect();
        fds.append(&mut consumers);

        fds
    }
}
