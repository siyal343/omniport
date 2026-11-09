use crate::MmapCaptureEngine;
use crate::pcapng::{PcapngError, PcapngWriter};
use omniport_core::SerialEvent;
use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ExtcapError {
    #[error("Failed to open extcap pipe or FIFO '{0}': {1}")]
    PipeOpen(String, std::io::Error),
    #[error("PCAPNG serialization error: {0}")]
    Pcapng(#[from] PcapngError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Generates stdout metadata for Wireshark `--extcap-interfaces` query
pub fn extcap_interfaces_output(version: &str) -> String {
    format!(
        "extcap {{version={}}}{{help=https://github.com/omniport/omniport}}\n\
         interface {{value=omniport}}{{display=OmniPort Virtual Serial Capture}}\n",
        version
    )
}

/// Generates stdout metadata for Wireshark `--extcap-dlts` query
pub fn extcap_dlts_output() -> String {
    "dlt {number=147}{name=USER0}{display=OmniPort Serial Frame (DLT 147)}\n\
     dlt {number=101}{name=RAW}{display=Raw Binary Frame (DLT 101)}\n\
     dlt {number=8}{name=SLIP}{display=RFC 1055 SLIP Frame (DLT 8)}\n"
        .to_string()
}

/// Generates stdout metadata for Wireshark `--extcap-config` query
pub fn extcap_config_output() -> String {
    "arg {number=0}{call=--endpoint}{display=Endpoint ID}{type=integer}{default=1}{range=1,65535}\n\
     arg {number=1}{call=--linktype}{display=Data Link Type}{type=selector}\n\
     value {arg=1}{value=147}{display=USER0 (147)}{default=true}\n\
     value {arg=1}{value=101}{display=Raw Binary (101)}{default=false}\n\
     value {arg=1}{value=8}{display=SLIP (8)}{default=false}\n"
        .to_string()
}

/// Real-time live PCAPNG streamer writing to a Wireshark named pipe (Windows) or FIFO (POSIX)
pub struct NamedPipeStreamer {
    writer: PcapngWriter<File>,
}

impl NamedPipeStreamer {
    /// Opens the Wireshark capture FIFO / named pipe and emits the initial PCAPNG headers
    pub fn open_pipe(fifo_path: &Path, link_type: u16) -> Result<Self, ExtcapError> {
        let file = OpenOptions::new()
            .write(true)
            .open(fifo_path)
            .map_err(|e| ExtcapError::PipeOpen(fifo_path.display().to_string(), e))?;

        let mut writer = PcapngWriter::with_link_type(file, link_type);
        writer.write_header("OmniPort Serial Capture")?;

        Ok(Self { writer })
    }

    pub fn from_writer(writer: PcapngWriter<File>) -> Self {
        Self { writer }
    }

    /// Writes a single packet to the Wireshark pipe
    pub fn write_packet(
        &mut self,
        timestamp_ns: u64,
        payload: &[u8],
        flags: u8,
    ) -> Result<(), ExtcapError> {
        self.writer.write_packet(timestamp_ns, payload, flags)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Writes a SerialEvent to the Wireshark pipe
    pub fn write_event(&mut self, event: &SerialEvent) -> Result<(), ExtcapError> {
        self.writer.write_event(event)?;
        self.writer.flush()?;
        Ok(())
    }

    /// Flushes the underlying writer
    pub fn flush(&mut self) -> Result<(), ExtcapError> {
        self.writer.flush()?;
        Ok(())
    }

    /// Streams live capture rows from an active `MmapCaptureEngine` until `stop` is signaled
    pub fn stream_mmap(
        &mut self,
        engine: &MmapCaptureEngine,
        mut current_row: u64,
        poll_interval: Duration,
        stop: &AtomicBool,
    ) -> Result<u64, ExtcapError> {
        let mut streamed_count = 0u64;
        let mut viewport_buf = Vec::with_capacity(65536);

        while !stop.load(Ordering::Relaxed) {
            let total_packets = engine.packet_count();
            if current_row < total_packets {
                let fetch_count = (total_packets - current_row).min(256) as usize;
                viewport_buf.clear();
                engine.read_viewport(current_row, fetch_count, &mut viewport_buf);

                // Decode viewport rows per ICD format:
                // Header (16B): [total: u64 (8B)][returned: u32 (4B)][reserved: u32 (4B)]
                if viewport_buf.len() >= 16 {
                    let returned_rows =
                        u32::from_le_bytes(viewport_buf[8..12].try_into().unwrap()) as usize;
                    let mut offset = 16;
                    for _ in 0..returned_rows {
                        if offset + 24 > viewport_buf.len() {
                            break;
                        }
                        let ts_ns = u64::from_le_bytes(
                            viewport_buf[offset..offset + 8].try_into().unwrap(),
                        );
                        let payload_len = u32::from_le_bytes(
                            viewport_buf[offset + 16..offset + 20].try_into().unwrap(),
                        ) as usize;
                        let flags = viewport_buf[offset + 22];
                        offset += 24;

                        if offset + payload_len <= viewport_buf.len() {
                            let payload = &viewport_buf[offset..offset + payload_len];
                            self.writer.write_packet(ts_ns, payload, flags)?;
                            offset += payload_len;
                            current_row += 1;
                            streamed_count += 1;
                        }
                    }
                    self.writer.flush()?;
                }
            } else {
                std::thread::sleep(poll_interval);
            }
        }

        Ok(streamed_count)
    }

    /// Asynchronously streams live SerialEvents from a Tokio bounded channel to Wireshark
    pub async fn stream_channel(
        &mut self,
        mut rx: tokio::sync::mpsc::Receiver<SerialEvent>,
    ) -> Result<u64, ExtcapError> {
        let mut count = 0u64;
        while let Some(event) = rx.recv().await {
            self.writer.write_event(&event)?;
            self.writer.flush()?;
            count += 1;
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pcapng::LINKTYPE_USER0;
    use tempfile::NamedTempFile;

    #[test]
    fn test_extcap_metadata_outputs() {
        let interfaces = extcap_interfaces_output("0.1.0");
        assert!(interfaces.contains("interface {value=omniport}"));
        assert!(interfaces.contains("OmniPort Virtual Serial Capture"));

        let dlts = extcap_dlts_output();
        assert!(dlts.contains("dlt {number=147}"));
        assert!(dlts.contains("dlt {number=101}"));

        let config = extcap_config_output();
        assert!(config.contains("arg {number=0}"));
        assert!(config.contains("arg {number=1}"));
    }

    #[test]
    fn test_named_pipe_streamer_file_write() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path().to_path_buf();

        let mut streamer = NamedPipeStreamer::open_pipe(&path, LINKTYPE_USER0).unwrap();
        streamer
            .write_packet(1_000_000_000, b"WIRESHARK_TEST_FRAME_1", 0)
            .unwrap();
        streamer
            .write_packet(1_000_001_000, b"WIRESHARK_TEST_FRAME_2", 1)
            .unwrap();
        streamer.flush().unwrap();

        let content = std::fs::read(&path).unwrap();
        assert!(content.len() > 60);

        let mut reader = crate::pcapng::PcapngReader::new(content.as_slice());
        let packets = reader.read_all_packets().unwrap();
        assert_eq!(packets.len(), 2);
        assert_eq!(&packets[0].payload[..], b"WIRESHARK_TEST_FRAME_1");
        assert_eq!(&packets[1].payload[..], b"WIRESHARK_TEST_FRAME_2");
    }
}
