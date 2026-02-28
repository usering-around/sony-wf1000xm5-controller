use crate::{ESCAPE_BYTE, ESCAPE_MASK, MESSAGE_HEADER, MESSAGE_TRAILER, MessageType, checksum};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EqualizerPreset {
    Off = 0x0,
    Bright = 0x10,
    Excited = 0x11,
    Mellow = 0x12,
    Relaxed = 0x13,
    Vocal = 0x14,
    TrebleBoost = 0x15,
    BassBoost = 0x16,
    Speech = 0x17,
    Manual = 0xa0,
    Custom1 = 0xa1,
    Custom2 = 0xa2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeablePreset {
    Manual = 0xa0,
    Custom1 = 0xa1,
    Custom2 = 0xa2,
}

impl TryFrom<EqualizerPreset> for ChangeablePreset {
    type Error = ();
    fn try_from(value: EqualizerPreset) -> Result<Self, Self::Error> {
        Ok(match value {
            EqualizerPreset::Manual => Self::Manual,
            EqualizerPreset::Custom1 => Self::Custom1,
            EqualizerPreset::Custom2 => Self::Custom2,
            _ => return Err(()),
        })
    }
}

impl EqualizerPreset {
    pub fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            0x0 => Self::Off,
            0x10 => Self::Bright,
            0x11 => Self::Excited,
            0x12 => Self::Mellow,
            0x13 => Self::Relaxed,
            0x14 => Self::Vocal,
            0x15 => Self::TrebleBoost,
            0x16 => Self::BassBoost,
            0x17 => Self::Speech,
            0xa0 => Self::Manual,
            0xa1 => Self::Custom1,
            0xa2 => Self::Custom2,
            _ => return None,
        })
    }
}

impl std::fmt::Display for EqualizerPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AncMode {
    Off,
    ActiveNoiseCanceling,
    AmbientSound,
}

#[derive(Clone, Copy, Debug)]
pub enum BatteryType {
    Headphones = 0x1,
    Case = 0xa,
}

impl BatteryType {
    pub fn from_byte(byte: u8) -> Option<Self> {
        Some(match byte {
            0x1 | 0x9 => Self::Headphones,
            0xa => Self::Case,
            _ => return None,
        })
    }
}

#[derive(Debug)]
pub enum Command {
    Init,
    Ack,
    AncSet {
        dragging_ambient_sound_slider: bool,
        mode: AncMode,
        ambient_sound_voice_passthrough: bool,
        ambient_sound_level: AmbientLevel,
    },
    GetAncStatus,

    ChangeEqualizerPreset {
        preset: EqualizerPreset,
    },
    ChangeEqualizerSetting {
        // the preset to change the equalizer settings for
        preset: ChangeablePreset,
        bass_level: EqBand,
        band_400: EqBand,
        band_1000: EqBand,
        band_2500: EqBand,
        band_6300: EqBand,
        band_16000: EqBand,
    },
    GetBatteryStatus {
        battery_type: BatteryType,
    },
    GetEqualizerSettings,
    GetCodec,
    SoundPressureMeasure {
        on: bool,
    },
    GetSoundPressure,
}
/// A newtype around u8, constrained to 0 up to 20.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmbientLevel {
    value: u8,
}
#[derive(Debug, thiserror::Error)]
pub enum AmbientLevelError {
    #[error("AmbientLevel must be in range of 0...20, got {}", .0)]
    OutOfRange(u8),
}
impl AmbientLevel {
    /// Fails if the value is greater than 20
    pub fn new(value: u8) -> Result<Self, AmbientLevelError> {
        if value <= 20 {
            Ok(Self { value })
        } else {
            Err(AmbientLevelError::OutOfRange(value))
        }
    }
}

/// A newtype around i8, constrained to -10 up to 10.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EqBand {
    value: i8,
}
#[derive(Debug, thiserror::Error)]
pub enum EqBandError {
    #[error("EqBand must be in range of -10...10, got {}", .0)]
    OutOfRange(i8),
}
impl EqBand {
    /// Fails if the value is not in the range of -10...10 (inclusive)
    pub fn new(value: i8) -> Result<Self, EqBandError> {
        if value.abs() <= 10 {
            Ok(Self { value })
        } else {
            Err(EqBandError::OutOfRange(value))
        }
    }
    pub fn encode(&self) -> u8 {
        (self.value + 10) as u8
    }
}

impl Command {
    const EQUALIZER_SET: u8 = 0x58;
    const ANC_SET: u8 = 0x68;
    const ANC_STATUS_GET: u8 = 0x66;
    const SUPPORTS_AMBIENT_SOUND_CONTROL_2: u8 = 0x17;
    const GET_BATTERY_STATUS: u8 = 0x22;
    const EQUALIZER_GET: u8 = 0x56;
    const CODEC_GET: u8 = 0x12;
    fn to_bytes(&self) -> Vec<u8> {
        match self {
            Self::Init => {
                vec![0, 0]
            }

            Self::Ack => {
                vec![]
            }

            Command::AncSet {
                dragging_ambient_sound_slider,
                mode,
                ambient_sound_voice_passthrough,
                ambient_sound_level,
            } => {
                let mut out = vec![
                    Self::ANC_SET,
                    Self::SUPPORTS_AMBIENT_SOUND_CONTROL_2,
                    if *dragging_ambient_sound_slider { 0 } else { 1 },
                ];
                if *mode == AncMode::Off {
                    out.push(0);
                } else {
                    out.push(1);
                }

                if *mode == AncMode::AmbientSound {
                    out.push(1);
                } else {
                    out.push(0);
                }

                out.push(if *ambient_sound_voice_passthrough {
                    1
                } else {
                    0
                });
                out.push(ambient_sound_level.value);
                out
            }

            Self::GetAncStatus => {
                vec![Self::ANC_STATUS_GET, Self::SUPPORTS_AMBIENT_SOUND_CONTROL_2]
            }

            Self::ChangeEqualizerPreset { preset } => {
                vec![Self::EQUALIZER_SET, 0, *preset as u8, 0]
            }
            Self::ChangeEqualizerSetting {
                preset,
                bass_level,
                band_400,
                band_1000,
                band_2500,
                band_6300,
                band_16000,
            } => {
                let data_size = 6; // bass level + 5 bands
                vec![
                    Self::EQUALIZER_SET,
                    0,
                    *preset as u8,
                    data_size,
                    bass_level.encode(),
                    band_400.encode(),
                    band_1000.encode(),
                    band_2500.encode(),
                    band_6300.encode(),
                    band_16000.encode(),
                ]
            }

            Self::GetBatteryStatus { battery_type } => {
                vec![Self::GET_BATTERY_STATUS, *battery_type as u8]
            }

            Self::GetEqualizerSettings => {
                vec![Self::EQUALIZER_GET, 0]
            }

            Self::GetCodec => {
                vec![Self::CODEC_GET, 2]
            }

            Self::SoundPressureMeasure { on } => {
                // from HCI logs start: 3e0e0000000004580301006e3c
                // from HCI logs stop: 3e0e0000000004580301016f3c
                vec![0x58, 0x03, 0x01, if *on { 0x00 } else { 0x01 }]
            }
            Self::GetSoundPressure => {
                // from HCI logs: 3e0e01000000025a036e3c
                vec![0x5a, 0x03]
            }
        }
    }
}

fn push_escaped(vec: &mut Vec<u8>, byte: u8) {
    if matches!(byte, MESSAGE_HEADER | MESSAGE_TRAILER | ESCAPE_BYTE) {
        vec.push(ESCAPE_BYTE);
        vec.push(byte & ESCAPE_MASK);
    } else {
        vec.push(byte);
    }
}

// this comment is taken from https://github.com/Freeyourgadget/Gadgetbridge/blob/master/app/src/main/java/nodomain/freeyourgadget/gadgetbridge/service/devices/sony/headphones/protocol/Message.java#L79
/**
 * Message format:
 * <p>
 * - MESSAGE_HEADER
 * - Message Type ({@link MessageType})
 * - Sequence Number - needs to be updated with the one sent in the ACK responses
 * - Payload Length - 4-byte big endian int with number of bytes that will follow
 * - N bytes of payload data (first being the PayloadType)
 * - Checksum (1-byte sum, excluding header)
 * - MESSAGE_TRAILER
 * <p>
 * Data between MESSAGE_HEADER and MESSAGE_TRAILER is escaped with MESSAGE_ESCAPE, and the
 * following byte masked with MESSAGE_ESCAPE_MASK.
 */
/// Build a command to send the headphones
pub fn build_command(command: &Command, seq_number: u8) -> Vec<u8> {
    let cmd = command.to_bytes();
    let mut buf = Vec::with_capacity(cmd.len() + 7);
    let message_type = match command {
        Command::AncSet { .. }
        | Command::GetCodec
        | Command::GetAncStatus
        | Command::ChangeEqualizerSetting { .. }
        | Command::ChangeEqualizerPreset { .. }
        | Command::Init
        | Command::GetBatteryStatus { .. }
        | Command::GetEqualizerSettings => MessageType::Command1,

        // from hci logs: SoundPressureMeasure: 3e0e0000000004580301006e3c
        // from hci log: GetSoundPressure: 3e0e01000000025a036e3c
        Command::SoundPressureMeasure { .. } | Command::GetSoundPressure => MessageType::Command2,

        Command::Ack => MessageType::Ack,
    };
    buf.push(message_type as u8);
    if matches!(command, Command::Ack) {
        buf.push(1u8.wrapping_sub(seq_number));
    } else {
        buf.push(seq_number);
    }
    buf.extend((cmd.len() as u32).to_be_bytes());
    buf.extend(cmd);
    buf.push(checksum(&buf));
    let mut out = Vec::with_capacity(buf.len() + 9);
    out.push(MESSAGE_HEADER);
    for byte in buf {
        push_escaped(&mut out, byte);
    }
    out.push(MESSAGE_TRAILER);

    out
}

#[cfg(test)]
mod test {
    use super::*;
    #[test]
    fn init() {
        // taken from hci logs
        let bytes = [0x3e, 0xc, 0x0, 0x0, 0x0, 0x0, 0x2, 0x0, 0x0, 0xe, 0x3c];
        assert_eq!(
            bytes.as_slice(),
            build_command(&Command::Init, 0).as_slice()
        );
    }
    #[test]
    fn init_ack() {
        // taken from hci logs
        let ack = [0x3e, 0x1, 0x0, 0x0, 0x0, 0x0, 0x0, 0x1, 0x3c];
        let init_seq_num = 1;
        let our_ack = build_command(&Command::Ack, init_seq_num);
        assert_eq!(ack.as_slice(), our_ack.as_slice());
    }
}
