use std::fmt;
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::thread::JoinHandle;
use std::time;
use std::time::Duration;

use rtipc::error::*;
use rtipc::client_connect;
use rtipc::ChannelVector;
use rtipc::ConsumeResult;
use rtipc::Consumer;
use rtipc::Producer;
use rtipc::{ChannelParam, VectorParam};

use rtipc::ProduceForceResult;
use rtipc::ProduceTryResult;

use crate::common::wait_pollin;
use crate::common::CommandId;
use crate::common::MsgCommand;
use crate::common::MsgEvent;
use crate::common::MsgResponse;

mod common;

static STOP_EVENT_LISTERNER: AtomicBool = AtomicBool::new(false);

fn handle_events(mut consumer: Consumer<MsgEvent>) -> Result<(), RtIpcError> {
    while !STOP_EVENT_LISTERNER.load(Ordering::Relaxed) {
        let eventfd = consumer.eventfd().unwrap();
        let ev = wait_pollin(eventfd, Duration::from_millis(10))?;

        if !ev {
            continue;
        }

        match consumer.pop() {
            ConsumeResult::Error => panic!(),
            ConsumeResult::NoMsgAvailable => { return Err(RtIpcError::Argument) }
            ConsumeResult::NoUpdate => { return Err(RtIpcError::Argument) }
            ConsumeResult::Success => {
                println!("client received event: {}", consumer.msg().unwrap())
            }
            ConsumeResult::MsgsDiscarded => {
                println!("client received event: {}", consumer.msg().unwrap())
            }
        };
    }
    println!("handle_events returns");
    Ok(())
}

struct App {
    command: Producer<MsgCommand>,
    response: Consumer<MsgResponse>,
    event_listener: Option<JoinHandle<Result<(), RtIpcError>>>,
}

impl App {
    pub fn new(mut vec: ChannelVector) -> Self {
        let command = vec.take_producer(0).unwrap();
        let response = vec.take_consumer(0).unwrap();
        let event = vec.take_consumer(1).unwrap();

        let event_listener = Some(thread::spawn(move || handle_events(event)));

        Self {
            command,
            response,
            event_listener,
        }
    }

    pub fn run(&mut self, cmds: &[MsgCommand]) {
        let pause = time::Duration::from_millis(10);

        for cmd in cmds {
            self.command.msg().clone_from(cmd);
            self.command.force_push();


            loop {
                match self.response.pop() {
                    ConsumeResult::Error => panic!(),
                    ConsumeResult::NoMsgAvailable => { thread::sleep(pause); continue; },
                    ConsumeResult::NoUpdate =>  { thread::sleep(pause); continue; },
                    ConsumeResult::Success => {}
                    ConsumeResult::MsgsDiscarded => {}
                };

                println!("client received response: {}", self.response.msg().unwrap());
                break;
            }
        }
        thread::sleep(time::Duration::from_millis(100));
        STOP_EVENT_LISTERNER.store(true, Ordering::Relaxed);
        self.event_listener.take().map(|h| h.join());
    }
}

fn main() {
    let commands: [MsgCommand; 6] = [
        MsgCommand {
            id: CommandId::Hello as u32,
            args: [1, 2, 0],
        },
        MsgCommand {
            id: CommandId::SendEvent as u32,
            args: [11, 20, 0],
        },
        MsgCommand {
            id: CommandId::SendEvent as u32,
            args: [12, 20, 1],
        },
        MsgCommand {
            id: CommandId::Div as u32,
            args: [100, 7, 0],
        },
        MsgCommand {
            id: CommandId::Div as u32,
            args: [100, 0, 0],
        },
        MsgCommand {
            id: CommandId::Stop as u32,
            args: [0, 0, 0],
        },
    ];

    let c2s_channels: [ChannelParam; 1] = [ChannelParam {
        add_msgs: 0,
        msg_size: unsafe { NonZeroUsize::new_unchecked(size_of::<MsgCommand>()) },
        eventfd: true,
        info: b"rpc command".to_vec(),
    }];

    let s2c_channels: [ChannelParam; 2] = [
        ChannelParam {
            add_msgs: 0,
            msg_size: unsafe { NonZeroUsize::new_unchecked(size_of::<MsgResponse>()) },
            eventfd: false,
            info:  b"rpc response".to_vec(),
        },
        ChannelParam {
            add_msgs: 10,
            msg_size: unsafe { NonZeroUsize::new_unchecked(size_of::<MsgEvent>()) },
            eventfd: true,
            info:  b"rpc event".to_vec(),
        },
    ];

    let vparam = VectorParam {
        producers: c2s_channels.to_vec(),
        consumers: s2c_channels.to_vec(),
        info:  b"rpc example".to_vec(),
    };
    let vec = client_connect("rtipc.sock", vparam).unwrap();
    let mut app = App::new(vec);
     thread::sleep(time::Duration::from_millis(100));
    app.run(&commands);
}
