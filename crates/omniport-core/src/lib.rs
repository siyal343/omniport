pub mod crc;
pub mod fault;
pub mod framing;
pub mod routing;

use bitflags::bitflags;
use bytes::Bytes;
use std::future::Future;
use std::pin::Pin;

pub type EndpointId = u16;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct PinState: u8 {
        const RTS   = 0b0000_0001;
        const CTS   = 0b0000_0010;
        const DTR   = 0b0000_0100;
        const DSR   = 0b0000_1000;
        const DCD   = 0b0001_0000;
        const RI    = 0b0010_0000;
        const BREAK = 0b0100_0000;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct FrameFlags: u8 {
        const DIR_TX       = 0b0000_0001;
        const ERR_PARITY   = 0b0000_0010;
        const ERR_FRAMING  = 0b0000_0100;
        const ERR_OVERRUN  = 0b0000_1000;
        const ERR_CRC      = 0b0001_0000;
        const FAULT_INJECT = 0b0010_0000;
        const PIN_CHANGE   = 0b0100_0000;
    }
}

#[derive(Debug, Clone)]
pub struct SerialEvent {
    pub timestamp_ns: u64,
    pub source_id: EndpointId,
    pub flags: FrameFlags,
    pub pins: PinState,
    pub payload: Bytes,
}

#[derive(Debug, Clone, Copy)]
pub struct SerialConfig {
    pub baud_rate: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: u8,
}

impl Default for SerialConfig {
    fn default() -> Self {
        Self {
            baud_rate: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: 0,
        }
    }
}

pub trait AsyncEndpoint: Send + Sync + 'static {
    fn id(&self) -> EndpointId;
    fn name(&self) -> &str;
    fn recv(&mut self) -> Pin<Box<dyn Future<Output = std::io::Result<SerialEvent>> + Send + '_>>;
    fn send(
        &mut self,
        event: SerialEvent,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send + '_>>;
    fn set_pins(&mut self, pins: PinState) -> std::io::Result<()>;
    fn configure(&mut self, cfg: SerialConfig) -> std::io::Result<()>;
}
