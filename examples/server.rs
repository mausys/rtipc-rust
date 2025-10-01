use nix::sys::socket::Backlog;

use std::fmt;
use std::{thread, time};

use rtipc::ChannelVector;
use rtipc::ConsumeResult;
use rtipc::Consumer;
use rtipc::Producer;

use rtipc::ProduceForceResult;
use rtipc::ProduceTryResult;

use rtipc::error::*;
use rtipc::Server;

use crate::common::CommandId;
use crate::common::MsgCommand;
use crate::common::MsgResponse;
use crate::common::MsgEvent;

mod common;

struct App {
    command: Consumer<MsgCommand>,
    response: Producer<MsgResponse>,
    event: Producer<MsgEvent>,
}

impl App {
    pub fn new(mut vec: ChannelVector) -> Self {
        let command = vec.take_consumer(0).unwrap();
        let response = vec.take_producer(0).unwrap();
        let event = vec.take_producer(1).unwrap();

        Self {
            command,
            response,
            event,
        }
    }
    fn run(&mut self) {
        let pause = time::Duration::from_millis(10);
        let mut run = true;
        let mut cnt = 0;

        while run {
            thread::sleep(pause);
            match self.command.pop() {
                ConsumeResult::Error => panic!(),
                ConsumeResult::NoMsgAvailable => continue,
                ConsumeResult::NoUpdate => continue,
                ConsumeResult::Success => {}
                ConsumeResult::MsgsDiscarded => {}
            };
            let cmd = self.command.msg().unwrap();
            self.response.msg().id = cmd.id;
            let args: [i32; 3] = cmd.args;
            println!("server received command: {}", cmd);

            let cmdid: CommandId = unsafe { ::std::mem::transmute(cmd.id) };
            self.response.msg().result = match cmdid {
                CommandId::Hello => 0,
                CommandId::Stop => {
                    run = false;
                    0
                }
                CommandId::SendEvent => {
                    self.send_events(args[0] as u32, args[1] as u32, args[2] != 0)
                }
                CommandId::Div => {
                    let (err, res) = self.div(args[0], args[1]);
                    self.response.msg().data = res;
                    err
                }
                _ => {
                    println!("unknown Command");
                    -1
                }
            };
            self.response.force_push();

            cnt = cnt + 1;
        }
    }
    fn send_events(&mut self, id: u32, num: u32, force: bool) -> i32 {
        for i in 0..num {
             println!("send_events {id} {i} {force}");
            let event = self.event.msg();
            event.id = id;
            event.nr = i;
            if force {
                self.event.force_push();
            } else {
                if self.event.try_push() == ProduceTryResult::Fail {
                    return i as i32;
                }
            }
        }
        num as i32
    }
    fn div(&mut self, a: i32, b: i32) -> (i32, i32) {
        if b == 0 {
            return (-1, 0);
        } else {
            return (0, a / b);
        }
    }
}

fn main() {
    let backlog = Backlog::new(1).unwrap();
    let server = Server::new("rtipc.sock", backlog).unwrap();
    let vec = server.accept().unwrap();
    let mut app = App::new(vec);
    app.run();
}
