use bytes::Bytes;
use omniport_core::{PinState, SerialConfig};

pub const IAC: u8 = 255;
pub const DONT: u8 = 254;
pub const DO: u8 = 253;
pub const WONT: u8 = 252;
pub const WILL: u8 = 251;
pub const SB: u8 = 250;
pub const SE: u8 = 240;

pub const COM_PORT_OPTION: u8 = 44;

// RFC 2217 Command Codes
pub const CMD_SET_BAUDRATE: u8 = 1;
pub const CMD_SET_DATASIZE: u8 = 2;
pub const CMD_SET_PARITY: u8 = 3;
pub const CMD_SET_STOPSIZE: u8 = 4;
pub const CMD_SET_CONTROL: u8 = 5;
pub const CMD_NOTIFY_MODEMSTATE: u8 = 107;

// RFC 2217 SET_CONTROL Subcodes
pub const CONTROL_BREAK_REQ: u8 = 5;
pub const CONTROL_BREAK_CLR: u8 = 6;
pub const CONTROL_DTR_ON: u8 = 8;
pub const CONTROL_DTR_OFF: u8 = 9;
pub const CONTROL_RTS_ON: u8 = 11;
pub const CONTROL_RTS_OFF: u8 = 12;

// RFC 2217 NOTIFY_MODEMSTATE Bitfield
pub const MODEM_CTS_MASK: u8 = 0x10;
pub const MODEM_DSR_MASK: u8 = 0x20;
pub const MODEM_RI_MASK: u8 = 0x40;
pub const MODEM_DCD_MASK: u8 = 0x80;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rfc2217Command {
    SetBaudrate(u32),
    NotifyBaudrate(u32),
    SetDataSize(u8),
    NotifyDataSize(u8),
    SetParity(u8),
    NotifyParity(u8),
    SetStopSize(u8),
    NotifyStopSize(u8),
    SetControl(u8),
    NotifyModemState(u8),
    RawSubneg { cmd: u8, data: Vec<u8> },
}

#[derive(Debug, PartialEq, Eq)]
enum ParserState {
    Data,
    Iac,
    Negotiation(u8), // DO, DONT, WILL, WONT
    Subneg,
    SubnegIac,
}

pub struct Rfc2217Codec {
    state: ParserState,
    subneg_buf: Vec<u8>,
}

impl Default for Rfc2217Codec {
    fn default() -> Self {
        Self::new()
    }
}

impl Rfc2217Codec {
    pub fn new() -> Self {
        Self {
            state: ParserState::Data,
            subneg_buf: Vec::with_capacity(64),
        }
    }
}

fn push_escaped(buf: &mut Vec<u8>, data: &[u8]) {
    for &b in data {
        if b == IAC {
            buf.push(IAC);
            buf.push(IAC);
        } else {
            buf.push(b);
        }
    }
}

impl Rfc2217Codec {
    /// Escapes raw serial payload bytes (replacing 0xFF with 0xFF 0xFF) per RFC 854
    pub fn encode_data(payload: &[u8]) -> Bytes {
        let mut out = Vec::with_capacity(payload.len() + 8);
        push_escaped(&mut out, payload);
        Bytes::from(out)
    }

    /// Encodes a Telnet RFC 2217 subnegotiation command `[IAC, SB, 44, cmd, ...data, IAC, SE]`
    pub fn encode_command(cmd: &Rfc2217Command) -> Bytes {
        let mut buf = Vec::with_capacity(16);
        buf.push(IAC);
        buf.push(SB);
        buf.push(COM_PORT_OPTION);

        match cmd {
            Rfc2217Command::SetBaudrate(br) => {
                buf.push(CMD_SET_BAUDRATE);
                push_escaped(&mut buf, &br.to_be_bytes());
            }
            Rfc2217Command::NotifyBaudrate(br) => {
                buf.push(CMD_SET_BAUDRATE + 100);
                push_escaped(&mut buf, &br.to_be_bytes());
            }
            Rfc2217Command::SetDataSize(ds) => {
                buf.push(CMD_SET_DATASIZE);
                push_escaped(&mut buf, &[*ds]);
            }
            Rfc2217Command::NotifyDataSize(ds) => {
                buf.push(CMD_SET_DATASIZE + 100);
                push_escaped(&mut buf, &[*ds]);
            }
            Rfc2217Command::SetParity(p) => {
                buf.push(CMD_SET_PARITY);
                push_escaped(&mut buf, &[*p]);
            }
            Rfc2217Command::NotifyParity(p) => {
                buf.push(CMD_SET_PARITY + 100);
                push_escaped(&mut buf, &[*p]);
            }
            Rfc2217Command::SetStopSize(ss) => {
                buf.push(CMD_SET_STOPSIZE);
                push_escaped(&mut buf, &[*ss]);
            }
            Rfc2217Command::NotifyStopSize(ss) => {
                buf.push(CMD_SET_STOPSIZE + 100);
                push_escaped(&mut buf, &[*ss]);
            }
            Rfc2217Command::SetControl(ctrl) => {
                buf.push(CMD_SET_CONTROL);
                push_escaped(&mut buf, &[*ctrl]);
            }
            Rfc2217Command::NotifyModemState(ms) => {
                buf.push(CMD_NOTIFY_MODEMSTATE);
                push_escaped(&mut buf, &[*ms]);
            }
            Rfc2217Command::RawSubneg { cmd, data } => {
                buf.push(*cmd);
                push_escaped(&mut buf, data);
            }
        }

        buf.push(IAC);
        buf.push(SE);
        Bytes::from(buf)
    }

    /// Encodes Telnet option negotiation (e.g. `[IAC, WILL, 44]`)
    pub fn encode_option(verb: u8, option: u8) -> Bytes {
        Bytes::copy_from_slice(&[IAC, verb, option])
    }

    /// Decodes a stream chunk, extracting unescaped data bytes and recognized RFC 2217 commands
    pub fn decode_chunk(
        &mut self,
        chunk: &[u8],
    ) -> (Vec<u8>, Vec<Rfc2217Command>, Vec<(u8, u8)>) {
        let mut clean_data = Vec::with_capacity(chunk.len());
        let mut commands = Vec::new();
        let mut negotiations = Vec::new();

        for &byte in chunk {
            match self.state {
                ParserState::Data => {
                    if byte == IAC {
                        self.state = ParserState::Iac;
                    } else {
                        clean_data.push(byte);
                    }
                }
                ParserState::Iac => match byte {
                    IAC => {
                        clean_data.push(IAC);
                        self.state = ParserState::Data;
                    }
                    WILL | WONT | DO | DONT => {
                        self.state = ParserState::Negotiation(byte);
                    }
                    SB => {
                        self.subneg_buf.clear();
                        self.state = ParserState::Subneg;
                    }
                    _ => {
                        self.state = ParserState::Data;
                    }
                },
                ParserState::Negotiation(verb) => {
                    negotiations.push((verb, byte));
                    self.state = ParserState::Data;
                }
                ParserState::Subneg => {
                    if byte == IAC {
                        self.state = ParserState::SubnegIac;
                    } else {
                        self.subneg_buf.push(byte);
                    }
                }
                ParserState::SubnegIac => {
                    if byte == SE {
                        // Subnegotiation ended: parse buffer
                        if let Some(cmd) = parse_subneg(&self.subneg_buf) {
                            commands.push(cmd);
                        }
                        self.subneg_buf.clear();
                        self.state = ParserState::Data;
                    } else if byte == IAC {
                        self.subneg_buf.push(IAC);
                        self.state = ParserState::Subneg;
                    } else {
                        self.state = ParserState::Subneg;
                    }
                }
            }
        }

        (clean_data, commands, negotiations)
    }
}

fn parse_subneg(buf: &[u8]) -> Option<Rfc2217Command> {
    if buf.is_empty() || buf[0] != COM_PORT_OPTION {
        return None;
    }
    if buf.len() < 2 {
        return None;
    }
    let cmd = buf[1];
    let payload = &buf[2..];

    match cmd {
        CMD_SET_BAUDRATE if payload.len() >= 4 => {
            let br = u32::from_be_bytes(payload[0..4].try_into().ok()?);
            Some(Rfc2217Command::SetBaudrate(br))
        }
        101 if payload.len() >= 4 => {
            let br = u32::from_be_bytes(payload[0..4].try_into().ok()?);
            Some(Rfc2217Command::NotifyBaudrate(br))
        }
        CMD_SET_DATASIZE if !payload.is_empty() => Some(Rfc2217Command::SetDataSize(payload[0])),
        102 if !payload.is_empty() => Some(Rfc2217Command::NotifyDataSize(payload[0])),
        CMD_SET_PARITY if !payload.is_empty() => Some(Rfc2217Command::SetParity(payload[0])),
        103 if !payload.is_empty() => Some(Rfc2217Command::NotifyParity(payload[0])),
        CMD_SET_STOPSIZE if !payload.is_empty() => Some(Rfc2217Command::SetStopSize(payload[0])),
        104 if !payload.is_empty() => Some(Rfc2217Command::NotifyStopSize(payload[0])),
        CMD_SET_CONTROL if !payload.is_empty() => Some(Rfc2217Command::SetControl(payload[0])),
        CMD_NOTIFY_MODEMSTATE if !payload.is_empty() => {
            Some(Rfc2217Command::NotifyModemState(payload[0]))
        }
        _ => Some(Rfc2217Command::RawSubneg {
            cmd,
            data: payload.to_vec(),
        }),
    }
}

/// Converts outbound `PinState` changes to RFC 2217 `SET_CONTROL` commands
pub fn pin_state_to_control(new_pins: PinState, old_pins: PinState) -> Vec<Rfc2217Command> {
    let mut cmds = Vec::new();

    if new_pins.contains(PinState::DTR) != old_pins.contains(PinState::DTR) {
        cmds.push(Rfc2217Command::SetControl(if new_pins.contains(PinState::DTR) {
            CONTROL_DTR_ON
        } else {
            CONTROL_DTR_OFF
        }));
    }

    if new_pins.contains(PinState::RTS) != old_pins.contains(PinState::RTS) {
        cmds.push(Rfc2217Command::SetControl(if new_pins.contains(PinState::RTS) {
            CONTROL_RTS_ON
        } else {
            CONTROL_RTS_OFF
        }));
    }

    if new_pins.contains(PinState::BREAK) != old_pins.contains(PinState::BREAK) {
        cmds.push(Rfc2217Command::SetControl(if new_pins.contains(PinState::BREAK) {
            CONTROL_BREAK_REQ
        } else {
            CONTROL_BREAK_CLR
        }));
    }

    cmds
}

/// Converts RFC 2217 modem state notification byte to `PinState`
pub fn modem_state_to_pin_state(modem_byte: u8, current_pins: PinState) -> PinState {
    let mut pins = current_pins;
    pins.set(PinState::CTS, (modem_byte & MODEM_CTS_MASK) != 0);
    pins.set(PinState::DSR, (modem_byte & MODEM_DSR_MASK) != 0);
    pins.set(PinState::RI, (modem_byte & MODEM_RI_MASK) != 0);
    pins.set(PinState::DCD, (modem_byte & MODEM_DCD_MASK) != 0);
    pins
}

/// Converts `SerialConfig` to RFC 2217 configuration commands
pub fn config_to_rfc2217_commands(cfg: &SerialConfig) -> Vec<Rfc2217Command> {
    vec![
        Rfc2217Command::SetBaudrate(cfg.baud_rate),
        Rfc2217Command::SetDataSize(cfg.data_bits),
        Rfc2217Command::SetParity(match cfg.parity {
            0 => 1, // None
            1 => 2, // Odd
            _ => 3, // Even
        }),
        Rfc2217Command::SetStopSize(if cfg.stop_bits >= 2 { 2 } else { 1 }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rfc854_iac_escaping() {
        let raw = [0x01, 0xFF, 0x02, 0xFF, 0x03];
        let encoded = Rfc2217Codec::encode_data(&raw);
        assert_eq!(&encoded[..], &[0x01, 0xFF, 0xFF, 0x02, 0xFF, 0xFF, 0x03]);

        let mut codec = Rfc2217Codec::new();
        let (decoded, cmds, negs) = codec.decode_chunk(&encoded);
        assert_eq!(&decoded[..], &raw);
        assert!(cmds.is_empty());
        assert!(negs.is_empty());
    }

    #[test]
    fn test_rfc2217_command_encoding_and_decoding() {
        let mut codec = Rfc2217Codec::new();

        // 1. Encode SET_BAUDRATE 115200
        let cmd = Rfc2217Command::SetBaudrate(115200);
        let bytes = Rfc2217Codec::encode_command(&cmd);
        assert_eq!(&bytes[..5], &[IAC, SB, COM_PORT_OPTION, CMD_SET_BAUDRATE, 0x00]);

        // 2. Decode it back
        let (data, cmds, _) = codec.decode_chunk(&bytes);
        assert!(data.is_empty());
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0], Rfc2217Command::SetBaudrate(115200));
    }

    #[test]
    fn test_modem_pin_state_mapping() {
        let modem_byte = MODEM_CTS_MASK | MODEM_DCD_MASK; // CTS + DCD active
        let pins = modem_state_to_pin_state(modem_byte, PinState::RTS | PinState::DTR);

        assert!(pins.contains(PinState::CTS));
        assert!(pins.contains(PinState::DCD));
        assert!(!pins.contains(PinState::DSR));
        assert!(!pins.contains(PinState::RI));
        assert!(pins.contains(PinState::RTS)); // Preserved
        assert!(pins.contains(PinState::DTR)); // Preserved
    }

    #[test]
    fn test_pin_state_to_control_commands() {
        let old_pins = PinState::empty();
        let new_pins = PinState::RTS | PinState::DTR;

        let cmds = pin_state_to_control(new_pins, old_pins);
        assert_eq!(cmds.len(), 2);
        assert!(cmds.contains(&Rfc2217Command::SetControl(CONTROL_DTR_ON)));
        assert!(cmds.contains(&Rfc2217Command::SetControl(CONTROL_RTS_ON)));
    }
}
