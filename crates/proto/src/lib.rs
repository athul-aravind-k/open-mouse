use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ProtoError {
    #[error("Serialization failed: {0}")]
    SerializeError(String),
    #[error("Deserialization failed: {0}")]
    DeserializeError(String),
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseBtn {
    Left,
    Right,
    Middle,
    X1,
    X2,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClipFormat {
    PlainText,
    Bitmap,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ScreenDescriptor {
    pub machine_id: String,
    pub width_px: u32,
    pub height_px: u32,
    pub dpi_scale: f32,
    pub offset_x: i32,
    pub offset_y: i32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum Packet {
    Hello {
        version: u32,
        machine_name: String,
        screen: ScreenDescriptor,
    },
    LayoutUpdate {
        screens: Vec<ScreenDescriptor>,
    },
    Heartbeat {
        timestamp_ms: u64,
    },
    HeartbeatAck {
        timestamp_ms: u64,
    },
    Enter {
        x_ratio: f32,
        y_ratio: f32,
    },
    Leave {
        x_ratio: f32,
        y_ratio: f32,
    },
    MouseMoveRel {
        dx: i32,
        dy: i32,
    },
    MouseButton {
        button: MouseBtn,
        pressed: bool,
    },
    MouseScroll {
        delta_x: i32,
        delta_y: i32,
    },
    Key {
        scancode: u16,
        extended: bool,
        pressed: bool,
    },
    ClipboardOffer {
        format: ClipFormat,
        data_len: usize,
        hash: u64,
    },
    ClipboardRequest {
        hash: u64,
    },
    ClipboardData {
        format: ClipFormat,
        hash: u64,
        payload: Vec<u8>,
    },
    EmergencyReset,
}

pub fn serialize_packet(packet: &Packet) -> Result<Vec<u8>, ProtoError> {
    postcard::to_allocvec(packet).map_err(|e| ProtoError::SerializeError(e.to_string()))
}

pub fn deserialize_packet(bytes: &[u8]) -> Result<Packet, ProtoError> {
    postcard::from_bytes(bytes).map_err(|e| ProtoError::DeserializeError(e.to_string()))
}

pub fn write_packet_framed<W: std::io::Write>(writer: &mut W, packet: &Packet) -> Result<(), ProtoError> {
    let payload = serialize_packet(packet)?;
    let len = payload.len() as u32;
    writer.write_all(&len.to_le_bytes()).map_err(|e| ProtoError::SerializeError(e.to_string()))?;
    writer.write_all(&payload).map_err(|e| ProtoError::SerializeError(e.to_string()))?;
    writer.flush().map_err(|e| ProtoError::SerializeError(e.to_string()))?;
    Ok(())
}

pub fn read_packet_framed<R: std::io::Read>(reader: &mut R) -> Result<Packet, ProtoError> {
    let mut len_bytes = [0u8; 4];
    reader.read_exact(&mut len_bytes).map_err(|e| ProtoError::DeserializeError(e.to_string()))?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    if len > 64 * 1024 * 1024 {
        return Err(ProtoError::DeserializeError(format!("Packet size {len} exceeds limit")));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).map_err(|e| ProtoError::DeserializeError(e.to_string()))?;
    deserialize_packet(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packet_roundtrip() {
        let packets = vec![
            Packet::Heartbeat { timestamp_ms: 123456789 },
            Packet::MouseMoveRel { dx: -15, dy: 30 },
            Packet::MouseButton {
                button: MouseBtn::Left,
                pressed: true,
            },
            Packet::Key {
                scancode: 0x1E, // 'A' key
                extended: false,
                pressed: true,
            },
            Packet::Enter {
                x_ratio: 0.0,
                y_ratio: 0.5,
            },
            Packet::EmergencyReset,
        ];

        for pkt in packets {
            let encoded = serialize_packet(&pkt).expect("serialization failed");
            let decoded = deserialize_packet(&encoded).expect("deserialization failed");
            assert_eq!(pkt, decoded);

            let mut cursor = std::io::Cursor::new(Vec::new());
            write_packet_framed(&mut cursor, &pkt).expect("write_packet_framed failed");
            cursor.set_position(0);
            let read_back = read_packet_framed(&mut cursor).expect("read_packet_framed failed");
            assert_eq!(pkt, read_back);
        }
    }
}

