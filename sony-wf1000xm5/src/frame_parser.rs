use thiserror::Error;

/// A parser which can parse the message format of headphones
/// and return a buffer containing the message.
pub struct FrameParser {
    msg_len: Option<usize>,
    buf: Vec<u8>,
    need_escape: bool,
}

pub enum FrameParserResult<'b> {
    /// We got the whole frame. You can parse buffer successfully
    /// Note that buf is already unescaped, so only parsing is necessary.
    Ready { buf: &'b [u8], consumed: usize },
    /// We need more bytes to complete the frame.
    /// If bytes_needed is Some, then it represents the amount of bytes needed until the completion of the frame.
    Incomplete { bytes_needed: Option<usize> },

    Error {
        err: FramerParserError,
        consumed: usize,
    },
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
                    buf: &self.buf,
                    consumed: idx + 1,
                };
            }
        }
        FrameParserResult::Incomplete {
            bytes_needed: self.bytes_needed(),
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
        } else if self.buf.len() == 1 || self.buf.len() == 2 {
            // we read the header, we now read the message type and seq number
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
        let bytes = vec![
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
        let good_messages = vec![
            bytes,
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
        for msg in good_messages {
            match parser.parse(&msg) {
                FrameParserResult::Ready { buf, consumed } => {
                    assert_eq!(consumed, msg.len());
                    assert_eq!(buf, msg);
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
    }
}
