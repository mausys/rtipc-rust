## RTIPC

**RTIPC** is a pure Rust implementation of rtipc: a zero-copy, wait-free inter-process communication (IPC) library optimized for real-time systems.

### Features
- **Zero-copy & syscall-free:** Extremely fast data transfer with no memory copying or system calls.
- **Deterministic behavior:** Data updates do not impact the runtime of the receiving process.
- **Real-time message handling:** Producers can send messages even when the queue is full—automatically discarding the oldest message to make room for the new one. This guarantees that the most recent data is always available.
- **SMP-optimized:** Messages are cacheline-aligned to minimize unnecessary cache coherence traffic in multi-core systems.
- **Event notification:** Optional *eventfd* support for integration with *select*, *poll*, and *epoll* event loops.
- **Multithreading:** Multiple threads can communicate concurrently over separate channels.

### Limitations
- **Fixed-size messages and queues:** Both the size of each message and the number of messages in a queue are fixed at creation time.

### Design
At its core, RTIPC uses a wait-free, zero-copy, single-producer single-consumer (SPSC) circular message queue. This queue allows a producer to overwrite the oldest message if the queue is full, ensuring real-time safety without blocking or performance degradation.

### How It Works

![alt text](https://github.com/mausys/rtipc-rust/blob/main/doc/flow.png)

1. The client initializes a channel vector based on desired parameters.
2. This channel vector contains one or more producer/consumer channels, all mapped onto a shared memory region.
3. The client shares the memory region and configuration (including optional metadata) with the server via a Unix socket.
4. The server then constructs its own matching channel vector, enabling seamless inter-process communication.
