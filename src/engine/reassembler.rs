use crate::engine::fetcher::{AppChunkFetcher, ChunkFetcher};
use crate::engine::types::AppChunkId;
use crate::engine::types::pb::seedvault::snapshot::Apk as ApkMeta;
use crate::util::path as safe_path;
use anyhow::{Context, Result};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::vec::IntoIter;

/// A reader that sequentially fetches and reassembles chunks.
///
/// Chunks are concatenated in the order provided.
pub struct ReassemblingReader<'a, F: ChunkFetcher> {
    fetcher: &'a F,
    chunk_ids: IntoIter<F::ChunkId>,
    current_buffer: Cursor<Vec<u8>>,
}

impl<'a, F: ChunkFetcher> ReassemblingReader<'a, F> {
    pub fn new(fetcher: &'a F, chunk_ids: Vec<F::ChunkId>) -> Self {
        Self {
            fetcher,
            chunk_ids: chunk_ids.into_iter(),
            current_buffer: Cursor::new(Vec::new()),
        }
    }
}

impl<'a, F: ChunkFetcher> Read for ReassemblingReader<'a, F> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.current_buffer.position() >= self.current_buffer.get_ref().len() as u64 {
            if let Some(next_id) = self.chunk_ids.next() {
                let data = self
                    .fetcher
                    .fetch_chunk(next_id)
                    .map_err(std::io::Error::other)?;
                self.current_buffer = Cursor::new(data);
            } else {
                return Ok(0);
            }
        }
        self.current_buffer.read(buf)
    }
}

/// Reassembles data by sequentially fetching and writing chunks to a writer.
///
/// Chunk IDs are expected to be in the correct concatenation order.
pub fn reassemble_data<F>(
    chunk_ids: Vec<F::ChunkId>,
    fetcher: &F,
    writer: &mut impl Write,
) -> Result<()>
where
    F: ChunkFetcher,
{
    let mut reader = ReassemblingReader::new(fetcher, chunk_ids);
    std::io::copy(&mut reader, writer)?;
    Ok(())
}

/// Reassembles and writes all splits of an APK to a directory.
pub fn reassemble_apk(apk_meta: &ApkMeta, fetcher: &AppChunkFetcher, out_dir: &Path) -> Result<()> {
    fs::create_dir_all(out_dir)
        .with_context(|| format!("Failed to create output directory for APK: {:?}", out_dir))?;

    for split in &apk_meta.splits {
        safe_path::validate_single_component(&split.name)?;
        let split_name_norm = safe_path::normalize_component(OsStr::new(&split.name));
        let filename = if split.name == "BASE_SPLIT" {
            "base.apk".to_string()
        } else {
            format!("{}.apk", split_name_norm.to_string_lossy())
        };
        let out_path = out_dir.join(&filename);

        let chunk_ids: Vec<AppChunkId> = split
            .chunk_ids
            .iter()
            .map(|bytes| {
                bytes
                    .as_slice()
                    .try_into()
                    .context("Failed to parse chunk ID for APK split")
            })
            .collect::<Result<_>>()?;

        let mut file = File::create(&out_path)
            .with_context(|| format!("Failed to create APK split file '{}'", out_path.display()))?;

        reassemble_data(chunk_ids, fetcher, &mut file)
            .with_context(|| format!("Failed to reassemble APK split '{}'", split.name))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
    struct MockChunkId(u8);

    impl std::fmt::Display for MockChunkId {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    struct MockFetcher {
        data: HashMap<u8, Vec<u8>>,
    }

    impl ChunkFetcher for MockFetcher {
        type ChunkId = MockChunkId;
        fn fetch_chunk(&self, id: MockChunkId) -> Result<Vec<u8>> {
            self.data
                .get(&id.0)
                .cloned()
                .ok_or(anyhow::anyhow!("Missing"))
        }
    }

    #[test]
    fn test_reader_crosses_boundaries() {
        let mut map = HashMap::new();
        map.insert(1, vec![0xAA, 0xBB]);
        map.insert(2, vec![0xCC]);
        map.insert(3, vec![0xDD, 0xEE, 0xFF]);

        let fetcher = MockFetcher { data: map };
        let reader = ReassemblingReader::new(
            &fetcher,
            vec![MockChunkId(1), MockChunkId(2), MockChunkId(3)],
        );

        let mut reader = reader;
        let mut output = Vec::new();
        reader.read_to_end(&mut output).unwrap();

        assert_eq!(output, vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    }
}
