use std::fmt;

#[derive(Copy, Clone, Debug)]
pub enum CommandId {
    Hello,
    Stop,
    SendEvent,
    Div,
    Unknown,
}

#[derive(Copy, Clone, Debug)]
pub struct MsgCommand {
    pub id: CommandId,
    pub args: [i32; 3],
}

#[derive(Copy, Clone, Debug)]
pub struct MsgResponse {
    pub id: CommandId,
    pub result: i32,
    pub data: i32,
}

#[derive(Copy, Clone, Debug)]
pub struct MsgEvent {
    pub id: u32,
    pub nr: u32,
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
        writeln!(
            f,
            "id: {}\n\tresult: {}\n\tdata: {}",
            self.id as u32, self.result, self.data
        )
    }
}

impl fmt::Display for MsgEvent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        writeln!(f, "id: {}\n\tnr: {}", self.id, self.nr)
    }
}
