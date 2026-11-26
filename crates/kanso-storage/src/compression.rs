use anyhow::{Context, Result};
use fastcdc::v2020::FastCDC;
use serde::{Deserialize, Serialize};
use std::io::Cursor;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ContentChunk {
    pub offset: usize,
    pub length: usize,
    pub hash: String,
}

pub fn compress_zstd(data: &[u8], level: i32) -> Result<Vec<u8>> {
    zstd::stream::encode_all(Cursor::new(data), level).context("Zstd compression failed")
}

pub fn decompress_zstd(data: &[u8]) -> Result<Vec<u8>> {
    zstd::stream::decode_all(Cursor::new(data)).context("Zstd decompression failed")
}

pub fn hash_blake3(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

pub fn chunk_fastcdc(data: &[u8]) -> Vec<ContentChunk> {
    if data.is_empty() {
        return Vec::new();
    }

    let min_size = 4 * 1024;
    let avg_size = 16 * 1024;
    let max_size = 64 * 1024;

    FastCDC::new(data, min_size, avg_size, max_size)
        .map(|chunk| {
            let slice = &data[chunk.offset..chunk.offset + chunk.length];
            ContentChunk {
                offset: chunk.offset,
                length: chunk.length,
                hash: hash_blake3(slice),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zstd_roundtrip_compresses_repetitive_logs() {
        let log = "INFO compiling module kanso-core\n".repeat(200);
        let compressed = compress_zstd(log.as_bytes(), 3).expect("compress");
        assert!(compressed.len() < log.len() / 5);
        let restored = decompress_zstd(&compressed).expect("decompress");
        assert_eq!(restored, log.as_bytes());
    }

    #[test]
    fn fastcdc_produces_deduped_blake3_chunks() {
        let mut payload = Vec::with_capacity(96 * 1024);
        for i in 0..3072 {
            payload.extend_from_slice(format!("block-{i:06}-kanso-payload--\n").as_bytes());
        }
        let chunks_a = chunk_fastcdc(&payload);
        let chunks_b = chunk_fastcdc(&payload);
        assert!(!chunks_a.is_empty());
        assert_eq!(chunks_a, chunks_b);
        let total_len: usize = chunks_a.iter().map(|c| c.length).sum();
        assert_eq!(total_len, payload.len());
    }
}
