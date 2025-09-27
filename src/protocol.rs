use std::{
    mem::align_of,
    num::NonZeroUsize,
    slice::{from_raw_parts, from_raw_parts_mut},
};

use crate::{
    error::*,
    header::{check_header, write_header, HEADER_SIZE},
    mem_align, ChannelParam,
};

#[repr(C)]
struct ChannelEntry {
    add_msgs: u32,
    msg_size: u32,
    eventfd: u32,
    info_size: u32,
}

impl ChannelEntry {
    fn from_param(param: &ChannelParam) -> Self {
        Self {
            add_msgs: param.add_msgs as u32,
            msg_size: param.msg_size.get() as u32,
            eventfd: param.eventfd as u32,
            info_size: param.info.len() as u32,
        }
    }
}

struct ChannelTable<'a> {
    msg: &'a [u8],
    consumers: &'a [ChannelEntry],
    producers: &'a [ChannelEntry],
    vector_info_offset: usize,
    vector_info_size: usize,
}

impl ChannelEntry {
    pub(crate) fn to_param(
        &self,
        msg: &[u8],
        info_offset: usize,
    ) -> Result<ChannelParam, MemError> {
        let info_size = self.info_size as usize;

        if info_offset + info_size > msg.len() {
            return Err(MemError::Size);
        }

        if self.msg_size == 0 {
            return Err(MemError::Value);
        }

        let msg_size = NonZeroUsize::new(self.msg_size as usize).unwrap();

        let info = match info_size {
            0 => Vec::with_capacity(0),
            _ => msg[info_offset..info_offset + info_size].to_vec(),
        };

        Ok(ChannelParam {
            add_msgs: self.add_msgs as usize,
            msg_size,
            info,
            eventfd: self.eventfd != 0,
        })
    }
}

struct Layout {
    vector_info_offset: usize,
    num_channels: [usize; 2],
    channels: [usize; 2],
    vector_info: usize,
    channel_infos: usize,
    size: usize,
}

impl Layout {
    pub(self) fn calc(
        producers: &Vec<ChannelParam>,
        consumers: &Vec<ChannelParam>,
        info: &Vec<u8>,
    ) -> Self {
        let mut offset = HEADER_SIZE;

        offset = mem_align(offset, align_of::<u32>());
        let vector_info_offset = offset;
        offset += size_of::<u32>();

        let num_channels: [usize; 2] = [offset, offset + size_of::<u32>()];
        offset += 2 * size_of::<u32>();

        offset = mem_align(offset, align_of::<ChannelEntry>());

        let channels: [usize; 2] = [offset, consumers.len() * size_of::<ChannelEntry>()];
        offset += (producers.len() + consumers.len()) * size_of::<ChannelEntry>();

        let vector_info = offset;
        offset += info.len();

        let channel_infos = offset;

        for param in producers {
            offset += param.info.len();
        }

        for param in consumers {
            offset += param.info.len();
        }

        let size = offset;

        Self {
            vector_info_offset,
            num_channels,
            channels,
            vector_info,
            channel_infos,
            size,
        }
    }
}

fn msg_get_ptr<T>(msg: &[u8], offset: usize) -> Result<*const T, MemError> {
    if offset + size_of::<T>() > msg.len() {
        return Err(MemError::Size);
    }

    let ptr = unsafe { msg.as_ptr().byte_add(offset) as *const T };

    if !ptr.is_aligned() {
        return Err(MemError::Alignment);
    }

    Ok(ptr)
}

fn msg_read<T>(msg: &[u8], offset: usize) -> Result<T, MemError> {
    let ptr = msg_get_ptr::<T>(msg, offset)?;

    Ok(unsafe { ptr.read() })
}

fn msg_get_mut_ptr<T>(msg: &mut [u8], offset: usize) -> Result<*mut T, MemError> {
    if offset + size_of::<T>() > msg.len() {
        return Err(MemError::Size);
    }

    let ptr = unsafe { msg.as_mut_ptr().byte_add(offset) as *mut T };

    if !ptr.is_aligned() {
        return Err(MemError::Alignment);
    }

    Ok(ptr)
}

fn msg_write<T: Copy>(msg: &[u8], offset: usize, val: &T) -> Result<(), MemError> {
    if offset + size_of::<T>() > msg.len() {
        return Err(MemError::Size);
    }

    let ptr = unsafe { msg.as_ptr().byte_add(offset) as *mut T };

    if !ptr.is_aligned() {
        return Err(MemError::Alignment);
    }

    unsafe {
        ptr.write(*val);
    }

    Ok(())
}

impl<'a> ChannelTable<'a> {
    pub(crate) fn from_msg(msg: &'a [u8]) -> Result<Self, CreateError> {
        let header = msg.get(0..HEADER_SIZE).ok_or(HeaderError::Size)?;

        check_header(header)?;

        let mut offset: usize = HEADER_SIZE;
        offset = mem_align(offset, align_of::<u32>());

        let vector_info_size = msg_read::<u32>(msg, offset)? as usize;
        offset += size_of::<u32>();

        let num_consumers = msg_read::<u32>(msg, offset)? as usize;
        offset += size_of::<u32>();

        let num_producers = msg_read::<u32>(msg, offset)? as usize;
        offset += size_of::<u32>();

        offset = mem_align(offset, align_of::<ChannelEntry>());

        let consumers_ptr = msg_get_ptr::<ChannelEntry>(msg, offset)?;
        offset += num_consumers * size_of::<u32>();

        let producers_ptr = msg_get_ptr::<ChannelEntry>(msg, offset)?;
        offset += num_producers * size_of::<u32>();

        let vector_info_offset = offset;

        if vector_info_offset + vector_info_size > msg.len() {
            return Err(CreateError::Argument);
        }

        let consumers = unsafe { from_raw_parts(consumers_ptr, num_consumers) };
        let producers = unsafe { from_raw_parts(producers_ptr, num_producers) };

        Ok(ChannelTable {
            msg,
            consumers,
            producers,
            vector_info_offset,
            vector_info_size,
        })
    }

    fn to_params(
        &self,
    ) -> Result<(Vec<ChannelParam>, Vec<ChannelParam>, Vec<u8>), CreateError> {
        let mut consumers: Vec<ChannelParam> = Vec::with_capacity(self.consumers.len());
        let mut producers: Vec<ChannelParam> = Vec::with_capacity(self.producers.len());

        let mut channel_info_offset = self.vector_info_offset + self.vector_info_size;

        let info: Vec<u8> = self.msg[self.vector_info_offset..channel_info_offset].to_vec();

        for entry in self.consumers {
            let param = entry.to_param(self.msg, channel_info_offset)?;
            channel_info_offset += param.info.len();
            consumers.push(param);
        }

        for entry in self.producers {
            let param = entry.to_param(self.msg, channel_info_offset)?;
            channel_info_offset += param.info.len();
            producers.push(param);
        }

        Ok((consumers, producers, info))
    }
}

pub fn parse_request_message(
    msg: &[u8],
) -> Result<(Vec<ChannelParam>, Vec<ChannelParam>, Vec<u8>), CreateError> {
    let table = ChannelTable::from_msg(msg)?;
    table.to_params()
}

pub(crate) fn create_request_message(
    producers: &Vec<ChannelParam>,
    consumers: &Vec<ChannelParam>,
    info: &Vec<u8>,
) -> Vec<u8> {
    let layout = Layout::calc(producers, consumers, info);

    let mut msg: Vec<u8> = vec![0; layout.size];

    write_header(msg.as_mut_slice());

    msg_write(
        msg.as_mut_slice(),
        layout.vector_info_offset,
        &(info.len() as u32),
    )
    .unwrap();

    msg_write(
        msg.as_mut_slice(),
        layout.num_channels[1],
        &(producers.len() as u32),
    )
    .unwrap();

    msg_write(
        msg.as_mut_slice(),
        layout.num_channels[0],
        &(consumers.len() as u32),
    )
    .unwrap();

    let producers_ptr =
        msg_get_mut_ptr::<ChannelEntry>(msg.as_mut_slice(), layout.channels[0]).unwrap();

    let consumers_ptr =
        msg_get_mut_ptr::<ChannelEntry>(msg.as_mut_slice(), layout.channels[1]).unwrap();

    let producer_entries = unsafe { from_raw_parts_mut(producers_ptr, producers.len()) };
    let consumer_entries = unsafe { from_raw_parts_mut(consumers_ptr, consumers.len()) };

    msg[layout.vector_info..layout.vector_info + info.len()].clone_from_slice(info);

    let mut info_offset = layout.channel_infos;

    for (index, param) in producers.iter().enumerate() {
        producer_entries[index] = ChannelEntry::from_param(param);

        if param.info.len() > 0 {
            msg[info_offset..info_offset + param.info.len()]
                .clone_from_slice(param.info.as_slice());
            info_offset += param.info.len();
        }
    }

    for (index, param) in consumers.iter().enumerate() {
        consumer_entries[index] = ChannelEntry::from_param(param);

        if param.info.len() > 0 {
            msg[info_offset..info_offset + param.info.len()]
                .clone_from_slice(param.info.as_slice());
            info_offset += param.info.len();
        }
    }

    msg
}
