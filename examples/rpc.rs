use rtipc::ChannelParam;
use rtipc::Consumer;
use rtipc::Producer;
use rtipc::RtIpc;
use std::fmt;
use std::mem::size_of;
use std::num::NonZeroUsize;
use std::os::fd::OwnedFd;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::{thread, time};

#[derive(Copy, Clone, Debug)]
enum CommandId {
    Hello,
    Stop,
    SendEvent,
    Div,
    Unknown,
}

#[derive(Copy, Clone, Debug)]
struct MsgCommand {
    id: CommandId,
    args: [i32; 3],
}

#[derive(Copy, Clone, Debug)]
struct MsgResponse {
    id: CommandId,
    result: i32,
    data: i32,
}

#[derive(Copy, Clone, Debug)]
struct MsgEvent {
    id: u32,
    nr: u32,
}

impl fmt::Display for MsgCommand {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "id: {}", self.id as u32)?;
        for (idx, arg) in self.args.iter().enumerate() {
            writeln!(f, "\targ[{}]: {}", idx, arg)?
        }
        Ok(())
    }
}

impl fmt::Display for MsgResponse {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "id: {}\n\tresult: {}\n\tdata: {}", self.id as u32, self.result, self.data)
    }
}

impl fmt::Display for MsgEvent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "id: {}\n\tnr: {}", self.id, self.nr)
    }
}

const COOKIE: u32 = 0x13579bdf;

const CLIENT2SERVER_CHANNELS: [ChannelParam; 1] = [ChannelParam {
    add_msgs: 0,
    msg_size: unsafe { NonZeroUsize::new_unchecked(size_of::<MsgCommand>()) },
}];

const SERVER2CLIENT_CHANNELS: [ChannelParam; 2] = [
    ChannelParam {
        add_msgs: 0,
        msg_size: unsafe { NonZeroUsize::new_unchecked(size_of::<MsgResponse>()) },
    },
    ChannelParam {
        add_msgs: 10,
        msg_size: unsafe { NonZeroUsize::new_unchecked(size_of::<MsgEvent>()) },
    },
];

struct Client {
    command: Producer<MsgCommand>,
    response: Consumer<MsgResponse>,
    event: Consumer<MsgEvent>,
}

impl Client {
    pub fn new(fd: OwnedFd) -> Self {
        let mut rtipc = RtIpc::from_fd(fd, COOKIE).unwrap();
        let command = rtipc.take_producer(0).unwrap();
        let response = rtipc.take_consumer(0).unwrap();
        let event = rtipc.take_consumer(1).unwrap();
        Client {
            command,
            response,
            event,
        }
    }

    pub fn run(&mut self, cmds: &[MsgCommand]) {
        let pause = time::Duration::from_millis(100);

        for cmd in cmds {
            self.command.msg().clone_from(cmd);
            self.command.force_put();

            thread::sleep(pause);
            loop {
                let rsp = self.response.fetch_tail();
                if let Some(rsp) = rsp {
                    println!("client received response: {}", rsp);
                } else {
                    break;
                }
            }
            loop {
                let event = self.event.fetch_tail();
                if let Some(event) = event {
                    println!("client received event: {}", event);
                } else {
                    break;
                }
            }
        }
    }
}

struct Server {
    command: Consumer<MsgCommand>,
    response: Producer<MsgResponse>,
    event: Producer<MsgEvent>,
    pub fd: OwnedFd,
}

impl Server {
    pub fn new() -> Server {
        let mut rtipc =
            RtIpc::new_anon_shm(&CLIENT2SERVER_CHANNELS, &SERVER2CLIENT_CHANNELS, COOKIE).unwrap();
        let rfd = rtipc.get_fd();
        let fd = rfd.try_clone().unwrap();
        let command = rtipc.take_consumer(0).unwrap();
        let response = rtipc.take_producer(0).unwrap();
        let event = rtipc.take_producer(1).unwrap();

        Server {
            fd,
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
            let cmd = self.command.fetch_tail();
            if let Some(cmd) = cmd {
                self.response.msg().id = cmd.id;
                let args: [i32;3] = cmd.args;
                println!("server received command: {}", cmd);

                self.response.msg().result = match cmd.id {
                    CommandId::Hello => 0,
                    CommandId::Stop => {
                        run = false;
                        0
                    }
                    CommandId::SendEvent => self.send_events(args[0] as u32, args[1] as u32, args[2] != 0),
                    CommandId::Div => {let (err, res) = self.div(args[0], args[1]); self.response.msg().data = res; err },
                    _ => {
                        println!("unknown Command");
                        -1
                    }
                };
                self.response.force_put();

                cnt = cnt + 1;
            };

            thread::sleep(pause);
        }
    }
    fn send_events(&mut self, id: u32, num: u32, force: bool) -> i32 {
        for i in 0..num {
            let event = self.event.msg();
            event.id = id;
            event.nr = i;
            if force {
                 self.event.force_put();
            } else {
                if !self.event.try_put() {
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

fn run_server(tx: Sender<OwnedFd>) {
    let mut server = Server::new();
    tx.send(server.fd.try_clone().unwrap()).unwrap();
    server.run();
}

fn run_client(rx: Receiver<OwnedFd>) {
    let fd = rx.recv().unwrap();
    let mut client = Client::new(fd);
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

    client.run(&commands);
}

fn main() {
    let (tx, rx) = channel::<OwnedFd>();

    let server_handle = thread::spawn(move || run_server(tx));
    let client_handle = thread::spawn(move || run_client(rx));
    server_handle.join().unwrap();
    client_handle.join().unwrap();
}
