pub mod command;
pub mod frame_parser;
pub mod payload;

const MESSAGE_HEADER: u8 = 0x3e;
const MESSAGE_TRAILER: u8 = 0x3c;
const ESCAPE_BYTE: u8 = 0x3d;
const ESCAPE_MASK: u8 = 0b11101111;

fn checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0, |acc, b| acc.wrapping_add(*b))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageType {
    Ack = 0x1,
    Command1 = 0xc,
    Command2 = 0xe,
}
impl MessageType {
    pub fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            0x1 => Self::Ack,
            0xc => Self::Command1,
            0xe => Self::Command2,
            _ => return None,
        })
    }
}
