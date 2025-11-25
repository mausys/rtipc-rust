use std::{
    num::NonZeroUsize,
    slice::from_raw_parts_mut,
};

use crate::{
    error::*,
    header::{verify_header, write_header, HEADER_SIZE},
    log::error,
    ChannelParam, VectorParam,
};

#[repr(C)]
struct ChannelEntry {
    additional_messages: u32,
    message_size: u32,
    eventfd: u32,
    info_size: u32,
}

impl ChannelEntry {
    fn from_param(param: &ChannelParam) -> Self {
        Self {
            additional_messages: param.additional_messages as u32,
            message_size: param.message_size.get() as u32,
            eventfd: param.eventfd as u32,
            info_size: param.info.len() as u32,
        }
    }
}

impl ChannelEntry {
    pub(crate) fn to_param(
        &self,
        message: &[u8],
        info_offset: usize,
    ) -> Result<ChannelParam, RequestPointerError> {
        let info_size = self.info_size as usize;

        if info_offset + info_size > message.len() {
            error!("request: info exceeds message size");
            return Err(RequestPointerError::OutOfBounds);
        }

        if self.message_size == 0 {
            error!("request: message size = 0 not allowed");
            return Err(RequestPointerError::OutOfBounds);
        }

        let message_size = NonZeroUsize::new(self.message_size as usize).unwrap();

        let info = match info_size {
            0 => Vec::with_capacity(0),
            _ => message[info_offset..info_offset + info_size].to_vec(),
        };

        Ok(ChannelParam {
            additional_messages: self.additional_messages as usize,
            message_size,
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
    pub(self) fn calc(vparam: &VectorParam) -> Self {
        let mut offset = HEADER_SIZE;

        let vector_info_offset = offset;
        offset += size_of::<u32>();

        let num_channels: [usize; 2] = [offset, offset + size_of::<u32>()];
        offset += 2 * size_of::<u32>();

        let channels: [usize; 2] = [
            offset,
            offset + vparam.producers.len() * size_of::<ChannelEntry>(),
        ];
        offset += (vparam.producers.len() + vparam.consumers.len()) * size_of::<ChannelEntry>();

        let vector_info = offset;
        offset += vparam.info.len();

        let channel_infos = offset;

        for param in &vparam.producers {
            offset += param.info.len();
        }

        for param in &vparam.consumers {
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

fn request_read<T>(request: &[u8], offset: usize) -> Result<T, RequestPointerError> {
    if offset + size_of::<T>() > request.len() {
        return Err(RequestPointerError::OutOfBounds);
    }

    let ptr = unsafe { request.as_ptr().byte_add(offset) as *const T };

    Ok(unsafe { ptr.read_unaligned() })
}

fn req_get_mut_ptr<T>(request: &mut [u8], offset: usize) -> Result<*mut T, RequestPointerError> {
    if offset + size_of::<T>() > request.len() {
        return Err(RequestPointerError::OutOfBounds);
    }

    let ptr = unsafe { request.as_mut_ptr().byte_add(offset) as *mut T };

    if !ptr.is_aligned() {
        return Err(RequestPointerError::Misalignment);
    }

    Ok(ptr)
}

fn request_write<T: Copy>(
    request: &[u8],
    offset: usize,
    val: &T,
) -> Result<(), RequestPointerError> {
    if offset + size_of::<T>() > request.len() {
        return Err(RequestPointerError::OutOfBounds);
    }

    let ptr = unsafe { request.as_ptr().byte_add(offset) as *mut T };

    if !ptr.is_aligned() {
        return Err(RequestPointerError::Misalignment);
    }

    unsafe {
        ptr.write(*val);
    }

    Ok(())
}

pub(crate) fn parse_request(request: &[u8]) -> Result<VectorParam, ProcessRequestError> {
    let header = request
        .get(0..HEADER_SIZE)
        .ok_or(ProcessRequestError::RequestPointerError(
            RequestPointerError::OutOfBounds,
        ))?;

    verify_header(header).inspect_err(|e| {
        error!("parse header failed {e:?}");
    })?;

    let mut offset: usize = HEADER_SIZE;

    let vector_info_size = request_read::<u32>(request, offset).inspect_err(|_| {
        error!("request message too short");
    })? as usize;
    offset += size_of::<u32>();

    let num_consumers = request_read::<u32>(request, offset).inspect_err(|_| {
        error!("request message too small");
    })? as usize;
    offset += size_of::<u32>();

    let num_producers = request_read::<u32>(request, offset).inspect_err(|_| {
        error!("request message too small");
    })? as usize;
    offset += size_of::<u32>();

    let vector_info_offset = offset + (num_consumers + num_producers) * size_of::<ChannelEntry>();

    let mut channel_info_offset = vector_info_offset + vector_info_size;

    if channel_info_offset > request.len() {
        error!("request message too small for vector info");
        return Err(ProcessRequestError::RequestPointerError(
            RequestPointerError::OutOfBounds,
        ));
    }

    let info: Vec<u8> = request[vector_info_offset..channel_info_offset].to_vec();

    let mut consumers: Vec<ChannelParam> = Vec::with_capacity(num_consumers);
    let mut producers: Vec<ChannelParam> = Vec::with_capacity(num_producers);

    for _ in 0..num_consumers {
        let entry = request_read::<ChannelEntry>(request, offset).inspect_err(|_| {
            error!("request message too short");
        })?;

        let param = entry
            .to_param(request, channel_info_offset)
            .inspect_err(|_| {
                error!("parse consumer entry {:?} failed", consumers.len());
            })?;

        offset += size_of::<ChannelEntry>();
        channel_info_offset += param.info.len();

        consumers.push(param);
    }

    for _ in 0..num_producers {
        let entry = request_read::<ChannelEntry>(request, offset).inspect_err(|_| {
            error!("request message too short");
        })?;

        let param = entry
            .to_param(request, channel_info_offset)
            .inspect_err(|_| {
                error!("parse producer entry {:?} failed", consumers.len());
            })?;

        offset += size_of::<ChannelEntry>();
        channel_info_offset += param.info.len();

        producers.push(param);
    }

    if channel_info_offset > request.len() {
        error!("request message too small for channel infos");
        return Err(ProcessRequestError::RequestPointerError(
            RequestPointerError::OutOfBounds,
        ));
    }

    Ok(VectorParam {
        consumers,
        producers,
        info,
    })
}

pub(crate) fn create_request_message(vparam: &VectorParam) -> Vec<u8> {
    let layout = Layout::calc(vparam);

    let mut request: Vec<u8> = vec![0; layout.size];

    write_header(request.as_mut_slice());

    request_write(
        request.as_mut_slice(),
        layout.vector_info_offset,
        &(vparam.info.len() as u32),
    )
    .unwrap();

    request_write(
        request.as_mut_slice(),
        layout.num_channels[0],
        &(vparam.producers.len() as u32),
    )
    .unwrap();

    request_write(
        request.as_mut_slice(),
        layout.num_channels[1],
        &(vparam.consumers.len() as u32),
    )
    .unwrap();

    let producers_ptr =
        req_get_mut_ptr::<ChannelEntry>(request.as_mut_slice(), layout.channels[0]).unwrap();

    let consumers_ptr =
        req_get_mut_ptr::<ChannelEntry>(request.as_mut_slice(), layout.channels[1]).unwrap();

    let producer_entries = unsafe { from_raw_parts_mut(producers_ptr, vparam.producers.len()) };
    let consumer_entries = unsafe { from_raw_parts_mut(consumers_ptr, vparam.consumers.len()) };

    request[layout.vector_info..layout.vector_info + vparam.info.len()]
        .clone_from_slice(vparam.info.as_slice());

    let mut info_offset = layout.channel_infos;

    for (index, param) in vparam.producers.iter().enumerate() {
        producer_entries[index] = ChannelEntry::from_param(param);

        if !param.info.is_empty() {
            request[info_offset..info_offset + param.info.len()]
                .clone_from_slice(param.info.as_slice());
            info_offset += param.info.len();
        }
    }

    for (index, param) in vparam.consumers.iter().enumerate() {
        consumer_entries[index] = ChannelEntry::from_param(param);

        if !param.info.is_empty() {
            request[info_offset..info_offset + param.info.len()]
                .clone_from_slice(param.info.as_slice());
            info_offset += param.info.len();
        }
    }

    request
}
