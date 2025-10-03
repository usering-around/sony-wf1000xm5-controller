use thiserror::Error;

use crate::{
    MessageType, checksum,
    command::{AncMode, BatteryType, EqualizerPreset},
};

#[derive(Debug)]
pub struct Message<'a> {
    pub kind: MessageType,
    pub payload: &'a [u8],
    pub seq_num: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadType {
    InitReply,
    BatteryLevel,
    BatteryLevelNotify,
    Equalizer,
    EqualizerNotify,
    AncStatus,
    AncStatusNotify,
    CodecGet,
    CodecNotify,
}

impl PayloadType {
    pub fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            0x1 => Self::InitReply,
            0x13 => Self::CodecGet,
            0x15 => Self::CodecNotify,
            0x23 => Self::BatteryLevel,
            0x25 => Self::BatteryLevelNotify,
            0x57 => Self::Equalizer,
            0x59 => Self::EqualizerNotify,
            0x67 => Self::AncStatus,
            0x69 => Self::AncStatusNotify,

            _ => return None,
        })
    }
}

#[derive(Debug)]
pub enum BatteryLevel {
    Case(usize),
    Headphones { left: usize, right: usize },
}

#[derive(Clone, Copy, Debug)]
pub enum Codec {
    Unknown = 0,
    Sbc = 0x1,
    Aac = 0x2,
    Ldac = 0x10,
    Aptx = 0x20,
    AptxHd = 0x21,
}

impl Codec {
    pub fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            0 => Self::Unknown,
            0x1 => Self::Sbc,
            0x2 => Self::Aac,
            0x10 => Self::Ldac,
            0x20 => Self::Aptx,
            0x21 => Self::AptxHd,
            _ => return None,
        })
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Sbc => "SBC",
            Self::Aac => "AAC",
            Self::Ldac => "LDAC",
            Self::Aptx => "APTX",
            Self::AptxHd => "APTX HD",
        }
    }
}

#[derive(Debug)]
pub enum Payload {
    InitReply,
    BatteryLevel(BatteryLevel),
    Equalizer {
        preset: EqualizerPreset,
        clear_bass: i8,
        band_400: i8,
        band_1000: i8,
        band_2500: i8,
        band_6300: i8,
        band_16000: i8,
    },
    AncStatus {
        mode: AncMode,
        ambient_sound_voice_filtering: bool,
        ambient_sound_level: u8,
    },
    Codec {
        codec: Codec,
    },
}

#[derive(Debug, Error)]
pub enum ParseMessageError<'a> {
    #[error("checksum failed, got: 0x{got:x}, expected: 0x{expected:x}; message: {msg:?}")]
    ChecksumFailed {
        got: u8,
        expected: u8,
        msg: Message<'a>,
    },
    #[error("unknown message type: 0x{msg_type:x}; message: {msg:?}")]
    UnknownMessageType { msg_type: u8, msg: Message<'a> },

    #[error("Not enough bytes given. A proper message has at least 9 bytes.")]
    NotEnoughBytes,
}

/// Parse a message, giving a high level overview of its type, payload bytes, and sequential number.
/// Use parse_payload to get information about its payload.
/// Note: use on bytes you get with `FrameParserResult::Ready`
pub fn parse_message<'a>(
    bytes: &'a [u8],
) -> std::result::Result<Message<'a>, ParseMessageError<'a>> {
    // a proper message must be at least 9 bytes long
    // 1 header, 1 msg type, 1 seq num, 4 len, 1 checksum, 1 trailer
    if bytes.len() < 9 {
        return Err(ParseMessageError::NotEnoughBytes);
    }
    let Some(kind) = MessageType::from_byte(bytes[1]) else {
        return Err(ParseMessageError::UnknownMessageType {
            msg_type: bytes[1],
            msg: Message {
                kind: MessageType::Unknown,
                payload: &bytes[7..bytes.len() - 2],
                seq_num: bytes[2],
            },
        });
    };
    let msg = Message {
        kind,
        payload: &bytes[7..bytes.len() - 2],
        seq_num: bytes[2],
    };
    let supposed_checksum = bytes[bytes.len() - 2];
    let real_checksum = checksum(&bytes[1..bytes.len() - 2]);
    if supposed_checksum != real_checksum {
        return Err(ParseMessageError::ChecksumFailed {
            got: supposed_checksum,
            expected: real_checksum,
            msg,
        });
    }

    Ok(msg)
}

#[derive(Debug, Error)]
pub enum ParsePayloadError {
    #[error("The given payload is empty")]
    Empty,
    #[error("Unknown payload type: 0x{kind:x}")]
    UnknownPayloadType { kind: u8 },
    #[error("Unknown battery type: 0x{battery:x}")]
    UnknownBatteryType { battery: u8 },
    #[error("Unknown equalizer preset: 0x{preset:x}")]
    UnknownEqualizerPreset { preset: u8 },
    #[error("Unknown codec: 0x{codec:x}")]
    UnknownCodec { codec: u8 },
    #[error("Payload is too small for payload of type {payload_type:?}")]
    PayloadTooSmall { payload_type: PayloadType },
}

pub fn parse_payload(payload: &[u8]) -> std::result::Result<Payload, ParsePayloadError> {
    if payload.is_empty() {
        return Err(ParsePayloadError::Empty);
    }

    let payload_type = PayloadType::from_byte(payload[0])
        .ok_or(ParsePayloadError::UnknownPayloadType { kind: payload[0] })?;

    Ok(match payload_type {
        PayloadType::InitReply => Payload::InitReply,
        PayloadType::BatteryLevel | PayloadType::BatteryLevelNotify => {
            if payload.len() < 5 {
                return Err(ParsePayloadError::PayloadTooSmall { payload_type });
            }
            let battery_type = BatteryType::from_byte(payload[1]).ok_or(
                ParsePayloadError::UnknownBatteryType {
                    battery: payload[1],
                },
            )?;
            match battery_type {
                BatteryType::Case => Payload::BatteryLevel(BatteryLevel::Case(payload[2] as usize)),

                BatteryType::Headphones => Payload::BatteryLevel(BatteryLevel::Headphones {
                    left: payload[2] as usize,
                    right: payload[4] as usize,
                }),
            }
        }

        PayloadType::Equalizer | PayloadType::EqualizerNotify => {
            if payload.len() < 10 {
                return Err(ParsePayloadError::PayloadTooSmall { payload_type });
            }
            let clear_bass = payload[4] as i8 - 10;
            let band_400 = payload[5] as i8 - 10;
            let band_1000 = payload[6] as i8 - 10;
            let band_2500 = payload[7] as i8 - 10;
            let band_6300 = payload[8] as i8 - 10;
            let band_16000 = payload[9] as i8 - 10;
            Payload::Equalizer {
                preset: EqualizerPreset::from_byte(payload[2])
                    .ok_or(ParsePayloadError::UnknownEqualizerPreset { preset: payload[2] })?,
                clear_bass,
                band_400,
                band_1000,
                band_2500,
                band_6300,
                band_16000,
            }
        }

        PayloadType::AncStatus | PayloadType::AncStatusNotify => {
            if payload.len() < 7 {
                return Err(ParsePayloadError::PayloadTooSmall { payload_type });
            }
            let mode = if payload[3] == 0 {
                AncMode::Off
            } else if payload[4] == 0 {
                AncMode::ActiveNoiseCanceling
            } else {
                AncMode::AmbientSound
            };
            let ambient_sound_voice_filtering = payload[5] == 1;

            let ambient_sound_level = payload[6];

            Payload::AncStatus {
                mode,
                ambient_sound_voice_filtering,
                ambient_sound_level,
            }
        }

        PayloadType::CodecGet | PayloadType::CodecNotify => {
            if payload.len() < 3 {
                return Err(ParsePayloadError::PayloadTooSmall { payload_type });
            }

            let codec = Codec::from_byte(payload[2])
                .ok_or(ParsePayloadError::UnknownCodec { codec: payload[2] })?;
            Payload::Codec { codec }
        }
    })
}

#[cfg(test)]
mod test {
    use crate::frame_parser::{FrameParser, FrameParserResult};

    use super::*;

    #[test]
    fn parse_init_response() {
        // taken from hci logs
        let bytes = [0x3e, 0x1, 0x1, 0x0, 0x0, 0x0, 0x0, 0x2, 0x3c];
        let mut parser = FrameParser::new();
        match parser.parse(&bytes) {
            FrameParserResult::Ready { buf, consumed } => {
                println!("got: {:x?}", buf);
                let msg = parse_message(buf).unwrap();
                assert_eq!(msg.kind, MessageType::Ack);
                assert_eq!(msg.seq_num, 1);
                assert_eq!(msg.payload, []);
                assert_eq!(consumed, bytes.len());
            }

            FrameParserResult::Incomplete { bytes_needed } => {
                unreachable!("bytes needed: {:?}", bytes_needed);
            }

            FrameParserResult::Error { err, consumed } => {
                unreachable!("err: {err}, consumed: {consumed}");
            }
        }
    }
}
