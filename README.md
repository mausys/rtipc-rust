## RTIPC

**RTIPC** This is the pure Rust implementation of rtipc, a zero-copy, wait-free inter-process communication library suited for real-time systems.

### Features
- Extremely fast: no data-copying and no syscalls are used for a data transfer.
- Deterministic: data updates don't affect the runtime of the remote process.
- Real-Time: A producer can add a message to the queue even when it is full. In this case, the oldest message will be discarded to make room for the new one, ensuring that the most recent message remains available to the consumer.
- Optimized for SMP-systems: messages are cacheline aligned to avoid unneeded cache coherence transactions.
- Support for anonymous and named shared memory
- Multithreading support: multiple threads can communicate over different channels with each other.

### Limitations
- Messages and message queues are fixed-sized. 
- The library does not include notification or signaling.

### Design
The shared memory is divided into different channels. The core of the library is a single consumer single producer wait-free zero-copy circular message queue, that allows the producer to replace its oldest message with a new one.

#### Shared Memory Layout
|                 |
| --------------- |
| Header          |
|                 |
| Table           |
|                 |
| Channels        |

- Header: fixed size. Describes the memory layout. Written by the server during initization.
- Table: Each channel has a table entry. An entry contains the size and number of messages. The table is written by the server during initialization.
- Channels: Each channel has at least three equally sized message buffers plus atomic variables for exchange. For performance reasons the buffers are cacheline aligned.

