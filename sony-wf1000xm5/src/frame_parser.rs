use thiserror::Error;

use crate::{MessageType, checksum};

/// A parser which can parse the message format of headphones
/// and return a buffer containing the message.
pub struct FrameParser {
    msg_len: Option<usize>,
    buf: Vec<u8>,
    need_escape: bool,
}

pub enum FrameParserResult<'a> {
    /// We got the whole frame. You can parse buffer successfully
    /// Note that buf is already unescaped, so only parsing is necessary.
    Ready { msg: Message<'a>, consumed: usize },
    /// We need more bytes to complete the frame.
    /// If bytes_needed is Some, then it represents the amount of bytes needed until the completion of the frame.
    Incomplete { bytes_needed: Option<usize> },

    Error {
        err: FramerParserError,
        consumed: usize,
    },
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("Invalid checksum, got: 0x{got:x}, expected: 0x{expected:x}")]
pub struct InvalidChecksum {
    pub expected: u8,
    pub got: u8,
}

#[derive(Debug)]
pub struct Message<'a> {
    pub kind: Result<MessageType, u8>,
    pub seq_num: u8,
    pub payload: &'a [u8],
    pub checksum: Result<u8, InvalidChecksum>,
}

#[derive(Debug, Error)]
pub enum FramerParserError {
    #[error("The given bytes do not start with the MESSAGE_HEADER value.")]
    NoMessageHeader,
}
impl FrameParser {
    pub fn new() -> Self {
        Self {
            msg_len: None,
            buf: Vec::new(),
            need_escape: false,
        }
    }
    pub fn parse<'a>(&'a mut self, bytes: &[u8]) -> FrameParserResult<'a> {
        if self.done() {
            self.buf.clear();
        }
        for (idx, byte) in bytes.iter().enumerate() {
            if let Err(err) = self.parse_byte(*byte) {
                return FrameParserResult::Error {
                    err,
                    consumed: idx + 1,
                };
            }
            if self.done() {
                return FrameParserResult::Ready {
                    msg: Self::parse_message(&self.buf),
                    consumed: idx + 1,
                };
            }
        }
        FrameParserResult::Incomplete {
            bytes_needed: self.bytes_needed(),
        }
    }

    fn parse_message(buf: &'_ [u8]) -> Message<'_> {
        let kind = MessageType::from_byte(buf[1]).ok_or(buf[1]);
        let seq_num = buf[2];
        let supposed_checksum = buf[buf.len() - 2];
        let real_checksum = checksum(&buf[1..buf.len() - 2]);
        let checksum = if supposed_checksum == real_checksum {
            Ok(real_checksum)
        } else {
            Err(InvalidChecksum {
                expected: real_checksum,
                got: supposed_checksum,
            })
        };
        Message {
            kind,
            seq_num,
            payload: &buf[7..buf.len() - 2],
            checksum,
        }
    }

    fn done(&self) -> bool {
        self.bytes_needed().is_some_and(|n| n == 0)
    }
    fn bytes_needed(&self) -> Option<usize> {
        let msg_len = self.msg_len?;
        // +7 for the 7 bytes before the len, +2 for the 2 bytes after the payload
        Some(msg_len + 7 + 2 - self.buf.len())
    }
    fn parse_byte(&mut self, mut byte: u8) -> std::result::Result<(), FramerParserError> {
        if self.need_escape {
            byte |= !crate::ESCAPE_MASK;
            self.need_escape = false;
        } else if byte == crate::ESCAPE_BYTE {
            self.need_escape = true;
            return Ok(());
        }
        if self.buf.is_empty() {
            // byte must be Header
            if byte != crate::MESSAGE_HEADER {
                return Err(FramerParserError::NoMessageHeader);
            }
            self.buf.push(byte);
        } else if self.buf.len() == 1 {
            // we read the header, we now read the message type and seq number
            self.buf.push(byte);
        } else if self.buf.len() == 2 {
            // seq num
            self.buf.push(byte);
        } else if self.buf.len() >= 3 && self.buf.len() <= 6 {
            // we already read the message type and seq number, now we read the length
            self.buf.push(byte);
            if self.buf.len() == 7 {
                // we read all the length
                let len = u32::from_be_bytes([self.buf[3], self.buf[4], self.buf[5], self.buf[6]]);
                self.msg_len = Some(len as usize);
            }
        } else {
            self.buf.push(byte);
        }
        Ok(())
    }
}

impl Default for FrameParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod test {
    use crate::{
        MESSAGE_HEADER, MESSAGE_TRAILER,
        command::{AncMode, build_command},
    };

    use super::*;
    #[test]
    fn basic_messages() {
        let good_messages = vec![
            build_command(&crate::command::Command::GetAncStatus, 0),
            build_command(&crate::command::Command::GetEqualizerSettings, 0x69),
            build_command(
                &crate::command::Command::GetBatteryStatus {
                    battery_type: crate::command::BatteryType::Headphones,
                },
                0x22,
            ),
            build_command(
                &crate::command::Command::AncSet {
                    dragging_ambient_sound_slider: true,
                    mode: AncMode::AmbientSound,
                    ambient_sound_voice_filtering: false,
                    ambient_sound_level: 15,
                },
                0xe,
            ),
        ];
        let mut parser = FrameParser::new();
        for bytes in good_messages {
            match parser.parse(&bytes) {
                FrameParserResult::Ready { msg, consumed } => {
                    assert_eq!(msg.checksum, Ok(bytes[bytes.len() - 2]));
                    assert_eq!(msg.kind, Ok(MessageType::from_byte(bytes[1]).unwrap()));
                    assert_eq!(msg.seq_num, bytes[2]);
                    assert_eq!(consumed, bytes.len());
                    assert_eq!(bytes, parser.buf);
                }
                _ => panic!(
                    "bad; shouldn't have panicked! this message is theoritcally fine as far as the 'frame'  is concerned."
                ),
            }
        }
    }

    #[test]
    fn bad_msg() {
        let msg = vec![
            MESSAGE_HEADER + 2,
            0x1,
            0x1,
            0x0,
            0x0,
            0x0,
            0x0,
            0x0,
            MESSAGE_TRAILER,
        ];
        let mut parser = FrameParser::new();
        match parser.parse(&msg) {
            FrameParserResult::Error { err, consumed } => {
                println!("good: {err}, {consumed}");
            }
            _ => panic!("bad; shouldn't have panicked! t"),
        }

        let bad_checksum = vec![
            MESSAGE_HEADER,
            0x1,
            0x1,
            0x0,
            0x0,
            0x0,
            0x0,
            0x0,
            MESSAGE_TRAILER,
        ];
        match parser.parse(&bad_checksum) {
            FrameParserResult::Ready { msg, consumed } => {
                assert_eq!(consumed, bad_checksum.len());
                assert_eq!(msg.kind, Ok(MessageType::Ack));
                assert_eq!(
                    msg.checksum,
                    Err(InvalidChecksum {
                        expected: 2,
                        got: 0
                    })
                );
                assert_eq!(msg.seq_num, 1);
            }
            _ => panic!("bad; shouldn't have panicked! t"),
        }

        let bad_msg_type = vec![
            MESSAGE_HEADER,
            0x32,
            0x2,
            0x0,
            0x0,
            0x0,
            0x0,
            0x34,
            MESSAGE_TRAILER,
        ];
        match parser.parse(&bad_msg_type) {
            FrameParserResult::Ready { msg, consumed } => {
                assert_eq!(consumed, bad_msg_type.len());
                assert_eq!(msg.kind, Err(0x32));
                assert_eq!(msg.checksum, Ok(bad_msg_type[bad_msg_type.len() - 2]),);
                assert_eq!(msg.seq_num, bad_msg_type[2]);
            }

            FrameParserResult::Incomplete { bytes_needed } => {
                panic!("incomplete message? {bytes_needed:?}");
            }

            FrameParserResult::Error { err, consumed } => {
                panic!("error? {err:?}, consumed: {consumed}")
            }
        }
    }
}
