//! Minimal base64 decoding for WireGuard public key validation.

use anyhow::{bail, Result};

pub(crate) fn decode_base64(input: &str) -> Result<Vec<u8>> {
    let bytes = input.as_bytes();
    if input.is_empty() || !bytes.chunks_exact(4).remainder().is_empty() {
        bail!("invalid base64 length");
    }

    let mut output = Vec::new();
    let mut saw_padding = false;

    for chunk in bytes.chunks(4) {
        let mut values = [0u8; 4];
        let mut padding = 0;

        for (idx, byte) in chunk.iter().enumerate() {
            match *byte {
                b'A'..=b'Z' if !saw_padding => values[idx] = byte - b'A',
                b'a'..=b'z' if !saw_padding => values[idx] = byte - b'a' + 26,
                b'0'..=b'9' if !saw_padding => values[idx] = byte - b'0' + 52,
                b'+' if !saw_padding => values[idx] = 62,
                b'/' if !saw_padding => values[idx] = 63,
                b'=' => {
                    saw_padding = true;
                    padding += 1;
                    values[idx] = 0;
                }
                _ => bail!("invalid base64 character"),
            }
        }

        output.push((values[0] << 2) | (values[1] >> 4));
        if padding < 2 {
            output.push((values[1] << 4) | (values[2] >> 2));
        }
        if padding == 0 {
            output.push((values[2] << 6) | values[3]);
        }
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::PEER_KEY;

    #[test]
    fn decodes_wireguard_key_bytes() {
        let decoded = decode_base64(PEER_KEY).unwrap();

        assert_eq!(decoded.len(), 32);
        assert!(decoded.iter().any(|byte| *byte != 0));
    }

    #[test]
    fn rejects_invalid_characters_and_lengths() {
        assert!(decode_base64("not-a-key").is_err());
        assert!(decode_base64("abc").is_err());
        assert!(decode_base64("").is_err());
    }

    #[test]
    fn handles_standard_padding_variants() {
        assert_eq!(decode_base64("TQ==").unwrap(), b"M");
        assert_eq!(decode_base64("TWE=").unwrap(), b"Ma");
        assert_eq!(decode_base64("TWFu").unwrap(), b"Man");
    }
}
