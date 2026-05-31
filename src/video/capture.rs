use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use super::VideoError;

const MAGIC: &[u8; 8] = b"ARVCAP1\0";

#[derive(Clone)]
pub(super) struct CaptureWriter {
    inner: Arc<Mutex<CaptureState>>,
}

struct CaptureState {
    writer: BufWriter<File>,
    started_at: Option<Instant>,
}

impl CaptureWriter {
    pub(super) fn create(path: &Path) -> Result<Self, VideoError> {
        let mut writer = BufWriter::new(File::create(path)?);
        writer.write_all(MAGIC)?;

        Ok(Self {
            inner: Arc::new(Mutex::new(CaptureState {
                writer,
                started_at: None,
            })),
        })
    }

    pub(super) fn record(&self, kind: CapturePacketKind, payload: &[u8]) -> Result<(), VideoError> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| VideoError::new("video capture writer lock poisoned"))?;
        let started_at = *state.started_at.get_or_insert_with(Instant::now);
        let elapsed = started_at.elapsed();
        let payload_len = u32::try_from(payload.len())
            .map_err(|_| VideoError::new("video capture payload is too large"))?;

        state
            .writer
            .write_all(&(elapsed.as_nanos() as u64).to_le_bytes())?;
        state.writer.write_all(&[kind.to_byte()])?;
        state.writer.write_all(&payload_len.to_le_bytes())?;
        state.writer.write_all(payload)?;
        state.writer.flush()?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum CapturePacketKind {
    AvcC,
    HvcC,
    Payload,
}

impl CapturePacketKind {
    fn to_byte(self) -> u8 {
        match self {
            Self::AvcC => 1,
            Self::HvcC => 2,
            Self::Payload => 3,
        }
    }

    fn from_byte(value: u8) -> Result<Self, VideoError> {
        match value {
            1 => Ok(Self::AvcC),
            2 => Ok(Self::HvcC),
            3 => Ok(Self::Payload),
            _ => Err(VideoError::new("invalid video capture packet kind")),
        }
    }
}

pub(super) struct CapturedPacket {
    pub(super) elapsed: Duration,
    pub(super) kind: CapturePacketKind,
    pub(super) payload: Vec<u8>,
}

pub(super) fn read_capture(path: &Path) -> Result<Vec<CapturedPacket>, VideoError> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut magic = [0; MAGIC.len()];
    reader.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(VideoError::new("invalid video capture file header"));
    }

    let mut packets = Vec::new();
    loop {
        let mut elapsed = [0; 8];
        match reader.read_exact(&mut elapsed) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(error.into()),
        }

        let mut kind = [0; 1];
        let mut payload_len = [0; 4];
        reader.read_exact(&mut kind)?;
        reader.read_exact(&mut payload_len)?;

        let payload_len = u32::from_le_bytes(payload_len) as usize;
        let mut payload = vec![0; payload_len];
        reader.read_exact(&mut payload)?;

        packets.push(CapturedPacket {
            elapsed: Duration::from_nanos(u64::from_le_bytes(elapsed)),
            kind: CapturePacketKind::from_byte(kind[0])?,
            payload,
        });
    }

    Ok(packets)
}
