use std::fmt;
use std::num::NonZeroUsize;
use std::os::fd::OwnedFd;
use std::{thread, time};

use rtipc::client_connect;
use rtipc::ChannelVector;
use rtipc::ConsumeResult;
use rtipc::Consumer;
use rtipc::Producer;
use rtipc::{ChannelParam, VectorParam};

use rtipc::ProduceForceResult;
use rtipc::ProduceTryResult;


use crate::common::CommandId;
use crate::common::MsgCommand;
use crate::common::MsgResponse;
use crate::common::MsgEvent;

mod common;

const CLIENT2SERVER_CHANNELS: [ChannelParam; 1] = [ChannelParam {
    add_msgs: 0,
    msg_size: unsafe { NonZeroUsize::new_unchecked(size_of::<MsgCommand>()) },
    eventfd: true,
    info: vec![],
}];

const SERVER2CLIENT_CHANNELS: [ChannelParam; 2] = [
    ChannelParam {
        add_msgs: 0,
        msg_size: unsafe { NonZeroUsize::new_unchecked(size_of::<MsgResponse>()) },
        eventfd: true,
        info: vec![],
    },
    ChannelParam {
        add_msgs: 10,
        msg_size: unsafe { NonZeroUsize::new_unchecked(size_of::<MsgEvent>()) },
        eventfd: true,
        info: vec![],
    },
];

struct App {
    command: Producer<MsgCommand>,
    response: Consumer<MsgResponse>,
    event: Consumer<MsgEvent>,
}

impl App {
    pub fn new(mut vec: ChannelVector) -> Self {
        let command = vec.take_producer(0).unwrap();
        let response = vec.take_consumer(0).unwrap();
        let event = vec.take_consumer(1).unwrap();
        Self {
            command,
            response,
            event,
        }
    }

    pub fn run(&mut self, cmds: &[MsgCommand]) {
        let pause = time::Duration::from_millis(100);

        for cmd in cmds {
            self.command.msg().clone_from(cmd);
            self.command.force_push();

            thread::sleep(pause);
            loop {
                match self.response.pop() {
                    ConsumeResult::Error => panic!(),
                    ConsumeResult::NoMsgAvailable => break,
                    ConsumeResult::NoUpdate => break,
                    ConsumeResult::Success => {}
                    ConsumeResult::MsgsDiscarded => {}
                };

                println!("client received response: {}", self.response.msg().unwrap());
            }
            loop {
                match self.event.pop() {
                    ConsumeResult::Error => panic!(),
                    ConsumeResult::NoMsgAvailable => break,
                    ConsumeResult::NoUpdate => break,
                    ConsumeResult::Success => {}
                    ConsumeResult::MsgsDiscarded => {}
                };
                println!("client received event: {}", self.event.msg().unwrap());
            }
        }
    }
}

fn main() {
    let commands: [MsgCommand; 6] = [
        MsgCommand {
            id: CommandId::Hello,
            args: [1, 2, 0],
        },
        MsgCommand {
            id: CommandId::SendEvent,
            args: [11, 20, 0],
        },
        MsgCommand {
            id: CommandId::SendEvent,
            args: [12, 20, 1],
        },
        MsgCommand {
            id: CommandId::Div,
            args: [100, 7, 0],
        },
        MsgCommand {
            id: CommandId::Div,
            args: [100, 0, 0],
        },
        MsgCommand {
            id: CommandId::Stop,
            args: [0, 0, 0],
        },
    ];
    let vparam = VectorParam {
        producers: CLIENT2SERVER_CHANNELS.to_vec(),
        consumers: SERVER2CLIENT_CHANNELS.to_vec(),
        info: vec![],
    };
    let vec = client_connect("rtipc.sock", vparam).unwrap();
    let mut app = App::new(vec);
    app.run(&commands);
}
