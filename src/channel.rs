use std::num::NonZeroUsize;
use std::sync::atomic::Ordering;

use crate::cache::cacheline_aligned;
use crate::error::*;
use crate::shm::{Chunk, Span};



use crate::AtomicIndex;
use crate::ChannelParam;
use crate::Index;
use crate::MIN_MSGS;

const INVALID_INDEX: Index = Index::MAX;
const CONSUMED_FLAG: Index = Index::MAX - Index::MAX / 2;

const ORIGIN_MASK: Index = CONSUMED_FLAG;

const INDEX_MASK: Index = !ORIGIN_MASK;

pub enum FetchResult {
    None,
    Same,
    New,
}

struct Channel {
    _chunk: Chunk,
    msg_size: NonZeroUsize,
    head: *mut Index,
    tail: *mut Index,
    queue: Vec<*mut Index>,
    msgs: Vec<*mut ()>,
}

impl Channel {
    pub fn new(chunk: Chunk, param: &ChannelParam) -> Result<Channel, MemError> {
        let queue_len = param.add_msgs + MIN_MSGS;
        let index_size = size_of::<Index>();
        let queue_size = (2 + queue_len) * index_size;
        let msg_size = NonZeroUsize::new(cacheline_aligned(param.msg_size.get())).unwrap();

        let mut offset_index = 0;
        let mut offset_msg = cacheline_aligned(queue_size);

        let head: *mut Index = chunk.get_ptr(offset_index)?;
        offset_index += index_size;

        let tail: *mut Index = chunk.get_ptr(offset_index)?;
        offset_index += index_size;

        let mut queue: Vec<*mut Index> = Vec::with_capacity(queue_size);
        let mut msgs: Vec<*mut ()> = Vec::with_capacity(queue_size);

        for _ in 0..queue_len {
            let index: *mut Index = chunk.get_ptr(offset_index)?;
            let msg: *mut () = chunk.get_span_ptr(&Span {
                offset: offset_msg,
                size: msg_size,
            })?;

            queue.push(index);
            msgs.push(msg);

            offset_index += index_size;
            offset_msg += msg_size.get();
        }

        Ok(Channel {
            _chunk: chunk,
            msg_size: param.msg_size,
            head,
            tail,
            queue,
            msgs,
        })
    }

    pub(crate) fn init(&self) {
        self.tail_store(INVALID_INDEX);
        self.head_store(INVALID_INDEX);

        let last = self.queue.len() - 1;

        for idx in 0..last {
            self.queue_store(idx as Index, (idx + 1) as Index);
        }

        self.queue_store(last as Index, 0);
    }

    pub(self) fn msg_size(&self) -> NonZeroUsize {
        self.msg_size
    }

    fn tail(&self) -> &AtomicIndex {
        unsafe { AtomicIndex::from_ptr(self.tail) }
    }

    fn head(&self) -> &AtomicIndex {
        unsafe { AtomicIndex::from_ptr(self.head) }
    }

    fn queue(&self, idx: Index) -> &AtomicIndex {
        unsafe { AtomicIndex::from_ptr(self.queue[idx as usize]) }
    }

    pub(self) fn tail_load(&self) -> Index {
        self.tail().load(Ordering::SeqCst)
    }

    pub(self) fn tail_store(&self, val: Index) {
        self.tail().store(val, Ordering::SeqCst)
    }

    pub(self) fn tail_fetch_or(&self, val: Index) -> Index {
        self.tail().fetch_or(val, Ordering::SeqCst)
    }

    pub(self) fn tail_compare_exchange(&self, current: Index, new: Index) -> bool {
        self.tail()
            .compare_exchange(current, new, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    pub(self) fn head_load(&self) -> Index {
        self.head().load(Ordering::SeqCst)
    }

    pub(self) fn head_store(&self, val: Index) {
        self.head().store(val, Ordering::SeqCst);
    }

    pub(self) fn queue_load(&self, idx: Index) -> Index {
        self.queue(idx).load(Ordering::SeqCst)
    }

    pub(self) fn queue_store(&self, idx: Index, val: Index) {
        self.queue(idx).store(val, Ordering::SeqCst);
    }

    fn move_tail(&self, tail: Index) -> bool {
        let next = self.queue_load(tail & INDEX_MASK);
        self.tail_compare_exchange(tail, next)
    }
}

pub struct ProducerChannel {
    channel: Channel,
    head: Index, /* last message in chain that can be used by consumer, chain[head] is always INDEX_END */
    current: Index, /* message used by producer, will become head  */
    overrun: Index, /* message used by consumer when tail moved away by producer, will become current when released by consumer */
}

impl ProducerChannel {
    pub(crate) fn new(chunk: Chunk, param: &ChannelParam) -> Result<ProducerChannel, MemError> {
        let channel = Channel::new(chunk, param)?;
        Ok(ProducerChannel {
            channel,
            head: INVALID_INDEX,
            current: 0,
            overrun: INVALID_INDEX,
        })
    }

    pub(crate) fn init(&self) {
        self.channel.init();
    }

    pub(crate) fn msg_size(&self) -> NonZeroUsize {
        self.channel.msg_size()
    }

    pub(crate) fn current(&self) -> *mut () {
        let ptr = self.channel.msgs.get(self.current as usize).unwrap();
        ptr.cast()
    }

    /* set the next message as head
     * get_next(msgq, producer->current) after this call
     * will return INDEX_END */
    fn enqueue_msg(&mut self) {
        self.channel.queue_store(self.current, INVALID_INDEX);

        if self.head == INVALID_INDEX {
            self.channel.tail_store(self.current);
        } else {
            self.channel.queue_store(self.head, self.current);
        }

        self.head = self.current;

        self.channel.head_store(self.head);
    }

    /* try to jump over tail blocked by consumer */
    fn overrun(&mut self, tail: Index) -> bool {
        let channel = &mut self.channel;

        let new_current = channel.queue_load(tail & INDEX_MASK); /* next */
        let new_tail = channel.queue_load(new_current); /* after next */

        if channel
            .tail()
            .compare_exchange(tail, new_tail, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            self.overrun = tail & INDEX_MASK;
            self.current = new_current;
            true
        } else {
            /* consumer just released tail, so use it */
            self.current = tail & INDEX_MASK;
            false
        }
    }

    /* inserts the next message into the queue and
     * if the queue is full, discard the last message that is not
     * used by consumer. Returns pointer to new message */
    pub(crate) fn force_put(&mut self) -> bool {
        let mut discarded = false;

        let next = self.channel.queue_load(self.current);

        self.enqueue_msg();

        let tail = self.channel.tail_load();

        let consumed: bool = (tail & CONSUMED_FLAG) != 0;

        let full: bool = next == (tail & INDEX_MASK);

        if self.overrun != INVALID_INDEX {
            /* we overran the consumer and moved the tail, use overran message as
             * soon as the consumer releases it */
            if consumed {
                /* consumer released overrun message, so we can use it */
                /* requeue overrun */
                self.channel.queue_store(self.overrun, next);

                self.current = self.overrun;
                self.overrun = INVALID_INDEX;
            } else {
                /* consumer still blocks overran message, move the tail again,
                 * because the message queue is still full */
                if self.channel.move_tail(tail) {
                    self.current = tail & INDEX_MASK;
                    discarded = true;
                } else {
                    /* consumer just released overrun message, so we can use it */
                    /* requeue overrun */
                    self.channel.queue_store(self.overrun, next);

                    self.current = self.overrun;
                    self.overrun = INVALID_INDEX;
                }
            }
        } else {
            /* no previous overrun, use next or after next message */
            if !full {
                /* message queue not full, simply use next */
                self.current = next;
            } else if !consumed {
                /* message queue is full, but no message is consumed yet, so try to move tail */
                if self.channel.move_tail(tail) {
                    /* message queue is full -> tail & INDEX_MASK == next */
                    self.current = next;
                } else {
                    /*  consumer just started and consumed tail
                     *  we're assuming that consumer flagged tail (tail | CONSUMED_FLAG),
                     *  if this this is not the case, consumer already moved on
                     *  and we will use tail  */
                    discarded = self.overrun(tail | CONSUMED_FLAG);
                }
            } else {
                /* overrun the consumer, if the consumer keeps tail */
                discarded = self.overrun(tail);
            }
        }

        discarded
    }

    /* trys to insert the next message into the queue */
    pub(crate) fn try_put(&mut self) -> bool {
        let mut enqued = false;
        let next = self.channel.queue_load(self.current);

        let tail = self.channel.tail_load();

        let consumed = (tail & CONSUMED_FLAG) != 0;

        let full = next == (tail & INDEX_MASK);

        if self.overrun != INVALID_INDEX {
            if consumed {
                /* consumer released overrun message, so we can use it */
                /* requeue overrun */
                self.enqueue_msg();

                self.channel.queue_store(self.overrun, next);

                self.current = self.overrun;
                self.overrun = INVALID_INDEX;
                enqued = true;
            }
        } else {
            /* no previous overrun, use next or after next message */
            if !full {
                self.enqueue_msg();
                self.current = next;
                enqued = true;
            }
        }
        enqued
    }
}

pub struct ConsumerChannel {
    channel: Channel,
    current: Index,
}

impl ConsumerChannel {
    pub(crate) fn new(chunk: Chunk, param: &ChannelParam) -> Result<ConsumerChannel, MemError> {
        let channel = Channel::new(chunk, param)?;
        Ok(ConsumerChannel {
            channel,
            current: 0,
        })
    }

    pub(crate) fn init(&self) {
        self.channel.init();
    }

    pub(crate) fn msg_size(&self) -> NonZeroUsize {
        self.channel.msg_size()
    }

    pub(crate) fn current(&self) -> Option<*const ()> {
        let ptr = self.channel.msgs.get(self.current as usize)?;
        Some(ptr.cast())
    }

    pub(crate) fn fetch_head(&mut self) -> bool {
        loop {
            let tail = self.channel.tail_fetch_or(CONSUMED_FLAG);

            if tail == INVALID_INDEX {
                /* or CONSUMED_FLAG doesn't change INDEX_END*/
                return false;
            }

            let head = self.channel.head_load();

            if self
                .channel
                .tail_compare_exchange(tail | CONSUMED_FLAG, head | CONSUMED_FLAG)
            {
                /* only accept head if producer didn't move tail,
                 *  otherwise the producer could fill the whole queue and the head could be the
                 *  producers current message  */
                self.current = head;
                return true;
            }
        }
    }

    pub(crate) fn fetch_tail(&mut self) -> FetchResult {
        let tail = self.channel.tail_fetch_or(CONSUMED_FLAG);

        if tail == INVALID_INDEX {
            return FetchResult::None;
        }

        if tail & CONSUMED_FLAG != 0 {
            /* try to get next message */
            let next = self.channel.queue_load(self.current);

            if next == INVALID_INDEX {
                return FetchResult::Same;
            }

            if self
                .channel
                .tail_compare_exchange(tail, next | CONSUMED_FLAG)
            {
                self.current = next;
            } else {
                /* producer just moved tail, use it */
                self.current = self.channel.tail_fetch_or(CONSUMED_FLAG);
            }
        } else {
            /* producer moved tail, use it */
            self.current = tail;
        }

        FetchResult::New
    }
}
