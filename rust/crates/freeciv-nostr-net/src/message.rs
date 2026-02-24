//! Length-prefixed message framing for QUIC streams.

use crate::error::NetError;
use crate::protocol::{StreamId, LENGTH_PREFIX_SIZE, MAX_MESSAGE_SIZE};

/// A framed message with a stream ID and payload.
#[derive(Debug, Clone)]
pub struct FramedMessage {
    /// The type of stream this message belongs to.
    pub stream_id: StreamId,
    /// The raw payload bytes.
    pub payload: Vec<u8>,
}

/// Encode a message with length-prefix framing.
///
/// Format: `[1 byte stream_id] [4 bytes big-endian payload length] [payload]`
pub fn encode_message(msg: &FramedMessage) -> Vec<u8> {
    let len = msg.payload.len() as u32;
    let mut buf = Vec::with_capacity(1 + LENGTH_PREFIX_SIZE + msg.payload.len());
    buf.push(msg.stream_id as u8);
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&msg.payload);
    buf
}

/// Decode a length-prefixed message from a byte slice.
///
/// Returns the decoded message and the number of bytes consumed.
pub fn decode_message(data: &[u8]) -> Result<(FramedMessage, usize), NetError> {
    if data.len() < 1 + LENGTH_PREFIX_SIZE {
        return Err(NetError::IncompleteParse);
    }
    let stream_id = match data[0] {
        0 => StreamId::GameActions,
        1 => StreamId::StateSync,
        2 => StreamId::Chat,
        3 => StreamId::Heartbeat,
        other => return Err(NetError::InvalidStreamId(other)),
    };
    let len = u32::from_be_bytes([data[1], data[2], data[3], data[4]]) as usize;
    if len > MAX_MESSAGE_SIZE {
        return Err(NetError::MessageTooLarge(len));
    }
    let total = 1 + LENGTH_PREFIX_SIZE + len;
    if data.len() < total {
        return Err(NetError::IncompleteParse);
    }
    Ok((
        FramedMessage {
            stream_id,
            payload: data[5..total].to_vec(),
        },
        total,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let msg = FramedMessage {
            stream_id: StreamId::GameActions,
            payload: b"hello world".to_vec(),
        };
        let encoded = encode_message(&msg);
        let (decoded, consumed) = decode_message(&encoded).unwrap();
        assert_eq!(consumed, encoded.len());
        assert_eq!(decoded.stream_id, StreamId::GameActions);
        assert_eq!(decoded.payload, b"hello world");
    }

    #[test]
    fn roundtrip_all_stream_ids() {
        for (id, expected_byte) in [
            (StreamId::GameActions, 0u8),
            (StreamId::StateSync, 1),
            (StreamId::Chat, 2),
            (StreamId::Heartbeat, 3),
        ] {
            let msg = FramedMessage {
                stream_id: id,
                payload: vec![0xAB, 0xCD],
            };
            let encoded = encode_message(&msg);
            assert_eq!(encoded[0], expected_byte);
            let (decoded, _) = decode_message(&encoded).unwrap();
            assert_eq!(decoded.stream_id, id);
            assert_eq!(decoded.payload, vec![0xAB, 0xCD]);
        }
    }

    #[test]
    fn roundtrip_empty_payload() {
        let msg = FramedMessage {
            stream_id: StreamId::Heartbeat,
            payload: vec![],
        };
        let encoded = encode_message(&msg);
        assert_eq!(encoded.len(), 5); // 1 + 4 + 0
        let (decoded, consumed) = decode_message(&encoded).unwrap();
        assert_eq!(consumed, 5);
        assert_eq!(decoded.stream_id, StreamId::Heartbeat);
        assert!(decoded.payload.is_empty());
    }

    #[test]
    fn decode_too_short() {
        // Only 3 bytes — not enough for header
        let data = [0u8, 0, 0];
        let result = decode_message(&data);
        assert!(matches!(result, Err(NetError::IncompleteParse)));
    }

    #[test]
    fn decode_incomplete_payload() {
        // Header says 100 bytes of payload, but we only have 2
        let mut data = vec![0u8]; // stream id
        data.extend_from_slice(&100u32.to_be_bytes()); // length = 100
        data.extend_from_slice(&[0xAA, 0xBB]); // only 2 bytes
        let result = decode_message(&data);
        assert!(matches!(result, Err(NetError::IncompleteParse)));
    }

    #[test]
    fn decode_invalid_stream_id() {
        let mut data = vec![99u8]; // invalid stream id
        data.extend_from_slice(&4u32.to_be_bytes());
        data.extend_from_slice(&[1, 2, 3, 4]);
        let result = decode_message(&data);
        assert!(matches!(result, Err(NetError::InvalidStreamId(99))));
    }

    #[test]
    fn decode_message_too_large() {
        let huge_len = (MAX_MESSAGE_SIZE + 1) as u32;
        let mut data = vec![0u8]; // stream id
        data.extend_from_slice(&huge_len.to_be_bytes());
        // Don't need the actual payload bytes — error fires before reading them
        data.extend_from_slice(&[0u8; 16]);
        let result = decode_message(&data);
        assert!(matches!(result, Err(NetError::MessageTooLarge(_))));
    }

    #[test]
    fn encode_length_prefix_is_correct() {
        let msg = FramedMessage {
            stream_id: StreamId::StateSync,
            payload: vec![0u8; 300],
        };
        let encoded = encode_message(&msg);
        // Check length prefix bytes
        let len_bytes = &encoded[1..5];
        let decoded_len =
            u32::from_be_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]);
        assert_eq!(decoded_len, 300);
        assert_eq!(encoded.len(), 1 + 4 + 300);
    }
}
