use std::{fmt, mem::size_of, num::NonZeroUsize};

use crate::error::*;
use crate::shm::{Chunk, Span};
use crate::ChannelParam;

#[derive(Debug)]
pub(crate) struct ChannelEntry {
    pub(crate) param: ChannelParam,
    pub(crate) span: Span,
}

#[derive(Debug)]
pub(crate) struct ChannelTable {
    pub(crate) consumers: Vec<ChannelEntry>,
    pub(crate) producers: Vec<ChannelEntry>,
}

impl ChannelTable {
    pub(crate) fn new(
        param_consumers: &[ChannelParam],
        param_producers: &[ChannelParam],
        mut offset: usize,
    ) -> Self {
        let mut consumers: Vec<ChannelEntry> = Vec::with_capacity(param_consumers.len());
        let mut producers: Vec<ChannelEntry> = Vec::with_capacity(param_producers.len());

        for param in param_consumers {
            let size = param.size();
            let span = Span { offset, size };

            consumers.push(ChannelEntry {
                param: *param,
                span,
            });

            offset += size.get();
        }

        for param in param_producers {
            let size = param.size();
            let span = Span { size, offset };

            producers.push(ChannelEntry {
                param: *param,
                span,
            });

            offset += size.get();
        }

        ChannelTable {
            consumers,
            producers,
        }
    }

    fn calc_offset(idx: usize) -> usize {
        let entry_size = 2 * size_of::<u32>();
        idx * entry_size
    }

    fn read_u32(chunk: &Chunk, offset: usize) -> Result<u32, MemError> {
        let ptr: *const u32 = chunk.get_ptr(offset)?;
        let value = unsafe { ptr.read() };
        Ok(value)
    }

    fn write_u32(chunk: &Chunk, offset: usize, value: u32) -> Result<(), MemError> {
        let ptr: *mut u32 = chunk.get_ptr(offset)?;
        unsafe { ptr.write(value) };
        Ok(())
    }

    fn read_entry(chunk: &Chunk, idx: usize) -> Result<ChannelParam, MemError> {
        let offset = ChannelTable::calc_offset(idx);

        let add_msgs = ChannelTable::read_u32(chunk, offset)? as usize;
        let msg_size_raw = ChannelTable::read_u32(chunk, offset + size_of::<u32>())?;
        let msg_size = NonZeroUsize::new(msg_size_raw as usize).ok_or(MemError::Value)?;

        Ok(ChannelParam { add_msgs, msg_size })
    }

    fn write_entry(chunk: &Chunk, idx: usize, value: ChannelParam) -> Result<(), MemError> {
        let offset = ChannelTable::calc_offset(idx);

        let add_msgs: u32 = value.add_msgs as u32;
        let msg_size: u32 = value.msg_size.get() as u32;

        ChannelTable::write_u32(chunk, offset, add_msgs)?;
        ChannelTable::write_u32(chunk, offset + size_of::<u32>(), msg_size)
    }

    pub(crate) fn calc_size(num_channels: NonZeroUsize) -> NonZeroUsize {
        let size = ChannelTable::calc_offset(num_channels.get());
        NonZeroUsize::new(size).unwrap()
    }

    pub(crate) fn from_chunk(
        chunk: &Chunk,
        num_consumers: usize,
        num_producers: usize,
        mut offset: usize,
    ) -> Result<ChannelTable, MemError> {
        let mut consumers: Vec<ChannelEntry> = Vec::with_capacity(num_consumers);
        let mut producers: Vec<ChannelEntry> = Vec::with_capacity(num_producers);

        for idx in 0..num_producers {
            let param = ChannelTable::read_entry(chunk, idx)?;
            let size = param.size();
            let span = Span { offset, size };
            let entry = ChannelEntry { param, span };

            producers.push(entry);

            offset += size.get();
        }

        for idx in num_producers..(num_producers + num_consumers) {
            let param = ChannelTable::read_entry(chunk, idx)?;
            let size = param.size();
            let span = Span { offset, size };
            let entry = ChannelEntry { param, span };

            consumers.push(entry);

            offset += size.get();
        }
        Ok(ChannelTable {
            consumers,
            producers,
        })
    }

    pub(crate) fn write(&self, chunk: &Chunk) -> Result<(), MemError> {
        for (idx, entry) in self.consumers.iter().enumerate() {
            ChannelTable::write_entry(chunk, idx, entry.param)?;
        }

        let num_consumers = self.consumers.len();

        for (idx, entry) in self.producers.iter().enumerate() {
            ChannelTable::write_entry(chunk, num_consumers + idx, entry.param)?;
        }

        Ok(())
    }
}

impl fmt::Display for ChannelTable {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "consumers: {:?}\n producers: {:?}",
            self.consumers, self.producers
        )
    }
}
