//! Compression utilities for posting lists
//!
//! Implements VarInt encoding and Delta encoding to compress posting lists
//! and reduce disk space while maintaining fast decode performance.

/// Encode a u32 value using VarInt encoding
/// Returns the encoded bytes (1-5 bytes)
pub fn encode_varint(mut value: u32) -> Vec<u8> {
    let mut result = Vec::with_capacity(5);
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        result.push(byte);
        if value == 0 {
            break;
        }
    }
    result
}

/// Decode a VarInt from bytes
/// Returns (decoded value, bytes consumed)
pub fn decode_varint(bytes: &[u8]) -> Option<(u32, usize)> {
    let mut value = 0u32;
    let mut shift = 0;

    for (i, &byte) in bytes.iter().enumerate() {
        value |= ((byte & 0x7F) as u32) << shift;
        if byte & 0x80 == 0 {
            return Some((value, i + 1));
        }
        shift += 7;
        if shift >= 35 {
            return None; // Overflow
        }
    }
    None // Incomplete
}

/// Delta encode a sorted list of doc IDs
/// Converts [1, 5, 10, 15] to deltas [1, 4, 5, 5] then VarInt encodes
pub fn delta_encode(doc_ids: &[u32]) -> Vec<u8> {
    if doc_ids.is_empty() {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut prev = 0u32;

    for &doc_id in doc_ids {
        let delta = doc_id - prev;
        result.extend(encode_varint(delta));
        prev = doc_id;
    }

    result
}

/// Decode a delta-encoded posting list
pub fn delta_decode(bytes: &[u8]) -> Vec<u32> {
    let mut result = Vec::new();
    let mut prev = 0u32;
    let mut pos = 0;

    while pos < bytes.len() {
        match decode_varint(&bytes[pos..]) {
            Some((delta, consumed)) => {
                prev += delta;
                result.push(prev);
                pos += consumed;
            }
            None => break, // Corrupted data
        }
    }

    result
}

/// Encode a u64 value using VarInt encoding
pub fn encode_varint_u64(mut value: u64) -> Vec<u8> {
    let mut result = Vec::with_capacity(10);
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        result.push(byte);
        if value == 0 {
            break;
        }
    }
    result
}

/// Decode a u64 VarInt
pub fn decode_varint_u64(bytes: &[u8]) -> Option<(u64, usize)> {
    let mut value = 0u64;
    let mut shift = 0;

    for (i, &byte) in bytes.iter().enumerate() {
        value |= ((byte & 0x7F) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some((value, i + 1));
        }
        shift += 7;
        if shift >= 70 {
            return None; // Overflow
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_varint_small() {
        let encoded = encode_varint(127);
        assert_eq!(encoded.len(), 1);
        let (decoded, consumed) = decode_varint(&encoded).unwrap();
        assert_eq!(decoded, 127);
        assert_eq!(consumed, 1);
    }

    #[test]
    fn test_varint_large() {
        let encoded = encode_varint(300);
        assert_eq!(encoded.len(), 2);
        let (decoded, consumed) = decode_varint(&encoded).unwrap();
        assert_eq!(decoded, 300);
        assert_eq!(consumed, 2);
    }

    #[test]
    fn test_delta_encode() {
        let doc_ids = vec![1, 5, 10, 15];
        let encoded = delta_encode(&doc_ids);
        let decoded = delta_decode(&encoded);
        assert_eq!(decoded, doc_ids);
    }

    #[test]
    fn test_delta_empty() {
        let doc_ids: Vec<u32> = vec![];
        let encoded = delta_encode(&doc_ids);
        assert!(encoded.is_empty());
        let decoded = delta_decode(&encoded);
        assert!(decoded.is_empty());
    }
}
