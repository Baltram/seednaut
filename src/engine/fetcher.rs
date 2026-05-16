use crate::engine::crypto::{Key, create_streaming_aead};
use crate::engine::decrypt::{RepoFileType, decrypt_and_decompress};
use crate::engine::types::{
    AppChunkId, BlobId, FileChunkId, pb::seedvault::snapshot::Blob as AppBlobMeta,
};
use anyhow::{Context, Result, anyhow};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tink_core::StreamingAead;

/// A trait for fetching chunks of data by ID.
pub trait ChunkFetcher {
    type ChunkId: Display + Copy;

    fn fetch_chunk(&self, id: Self::ChunkId) -> Result<Vec<u8>>;
}
pub struct AppChunkFetcher {
    repo_path: PathBuf,
    aead: Box<dyn StreamingAead>,
    blobs: Arc<HashMap<String, AppBlobMeta>>,
}

impl AppChunkFetcher {
    /// Creates a new `AppChunkFetcher`.
    ///
    /// # Arguments
    ///
    /// * `repo_path` - The absolute path to the specific app repository directory (e.g., `.../.SeedVaultAndroidBackup/<64-char-hex-id>`).
    /// * `app_stream_key` - The derived 32-byte key for app data streams.
    /// * `blobs` - A map from `AppChunkId` (hex string) to blob metadata, taken from the snapshot.
    pub fn new(
        repo_path: &Path,
        app_stream_key: Key,
        blobs: Arc<HashMap<String, AppBlobMeta>>,
    ) -> Result<Self> {
        let aead =
            create_streaming_aead(app_stream_key).context("Failed to create AEAD primitive")?;
        Ok(Self {
            repo_path: repo_path.to_path_buf(),
            aead,
            blobs,
        })
    }
}

impl ChunkFetcher for AppChunkFetcher {
    type ChunkId = AppChunkId;

    /// Fetches a single app data chunk by its ID.
    ///
    /// Looks up the on-disk blob ID from snapshot metadata, reads the blob file,
    /// decrypts it, and verifies the content hash matches the request.
    fn fetch_chunk(&self, chunk_id: Self::ChunkId) -> Result<Vec<u8>> {
        let chunk_id_hex = chunk_id.to_string();

        let blob_meta = self.blobs.get(&chunk_id_hex).ok_or_else(|| {
            anyhow!(
                "Blob metadata not found in snapshot for chunk ID {}",
                chunk_id_hex
            )
        })?;

        let blob_id: BlobId = blob_meta
            .id
            .as_slice()
            .try_into()
            .with_context(|| format!("Invalid blob ID format for chunk {}", chunk_id_hex))?;
        let blob_id_hex = blob_id.to_string();

        let blob_path = self.repo_path.join(&blob_id_hex[..2]).join(&blob_id_hex);

        let decompressed_data =
            decrypt_and_decompress(&blob_path, &*self.aead, RepoFileType::AppChunk).with_context(
                || {
                    format!(
                        "Failed to decrypt chunk {}. The chunk file may be corrupted on disk.",
                        blob_path.display()
                    )
                },
            )?;

        // Verify the SHA256 hash of decrypted content matches the requested chunk ID.
        let actual_hash = Sha256::digest(&decompressed_data);
        if actual_hash.as_slice() != chunk_id.0.as_bytes() {
            return Err(anyhow!(
                "Content hash mismatch for chunk {}. Expected {}, got {}",
                chunk_id_hex,
                chunk_id_hex,
                hex::encode(actual_hash)
            ));
        }

        Ok(decompressed_data)
    }
}

/// Fetches, decrypts, and verifies chunks for a file backup.
pub struct FileChunkFetcher {
    repo_path: PathBuf,
    aead: Box<dyn StreamingAead>,
    chunk_id_key: Key,
}

impl FileChunkFetcher {
    /// Creates a new `FileChunkFetcher`.
    ///
    /// # Arguments
    ///
    /// * `repo_path` - The absolute path to the specific file repository directory (e.g., `.../.SeedVaultAndroidBackup/<16-char-hex-id>.sv`).
    /// * `file_stream_key` - The derived 32-byte key for file data streams.
    /// * `chunk_id_key` - The derived 32-byte key for calculating file chunk HMACs.
    pub fn new(repo_path: &Path, file_stream_key: Key, chunk_id_key: Key) -> Result<Self> {
        let aead =
            create_streaming_aead(file_stream_key).context("Failed to create AEAD primitive")?;
        Ok(Self {
            repo_path: repo_path.to_path_buf(),
            aead,
            chunk_id_key,
        })
    }
}

impl ChunkFetcher for FileChunkFetcher {
    type ChunkId = FileChunkId;

    /// Fetches a single file data chunk by its ID.
    ///
    /// Finds the chunk file on disk (filename = chunk ID), decrypts it,
    /// and verifies the content HMAC.
    fn fetch_chunk(&self, chunk_id: Self::ChunkId) -> Result<Vec<u8>> {
        let chunk_id_hex = chunk_id.to_string();

        let chunk_path = self.repo_path.join(&chunk_id_hex[..2]).join(&chunk_id_hex);

        let decrypted_data = decrypt_and_decompress(
            &chunk_path,
            &*self.aead,
            RepoFileType::FileChunk {
                chunk_id: chunk_id.0.as_bytes(),
            },
        )
        .with_context(|| {
            format!(
                "Failed to decrypt chunk {}. The chunk file may be corrupted on disk.",
                chunk_path.display()
            )
        })?;

        // Verify HMAC of decrypted content matches the requested chunk ID.
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.chunk_id_key)
            .expect("HMAC can take key of any size");
        mac.update(&decrypted_data);
        let result = mac.finalize();
        let actual_hmac = result.into_bytes();

        if actual_hmac.as_slice() != chunk_id.0.as_bytes() {
            return Err(anyhow!(
                "Content HMAC mismatch for chunk {}. Expected {}, got {}",
                chunk_id_hex,
                chunk_id_hex,
                hex::encode(actual_hmac)
            ));
        }

        Ok(decrypted_data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    #[derive(Clone)]
    struct SharedBuffer {
        data: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for SharedBuffer {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.data.lock().unwrap().write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_fetch_chunk_lookup_indirection() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let repo_path = temp_dir.path();
        let key = [0u8; 32];
        let aead = create_streaming_aead(key)?;

        let plaintext = b"Hello World";
        let chunk_id_hash = Sha256::digest(plaintext);
        let chunk_id_hex = hex::encode(chunk_id_hash);

        // Encrypt: the on-disk blob file format is [version_byte][encrypted_data],
        // where encrypted_data decrypts to [compressed_size:4BE][compressed_payload].
        let compressed = zstd::encode_all(&plaintext[..], 0)?;
        let mut payload_to_encrypt = Vec::new();
        payload_to_encrypt.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
        payload_to_encrypt.extend_from_slice(&compressed);

        let version_byte = 2u8;
        let ad = [version_byte];

        let buffer = Arc::new(Mutex::new(Vec::new()));
        buffer.lock().unwrap().push(version_byte);

        let writer_buf = SharedBuffer {
            data: buffer.clone(),
        };

        {
            let mut writer = aead
                .new_encrypting_writer(Box::new(writer_buf), &ad)
                .map_err(|e| anyhow::anyhow!("Encryption failed: {:?}", e))?;
            writer.write_all(&payload_to_encrypt)?;
        }

        let ciphertext = Arc::try_unwrap(buffer).unwrap().into_inner().unwrap();

        let blob_id_hash = Sha256::digest(&ciphertext);
        let blob_id_hex = hex::encode(blob_id_hash);

        let blob_dir = repo_path.join(&blob_id_hex[..2]);
        fs::create_dir_all(&blob_dir)?;
        fs::write(blob_dir.join(blob_id_hex.clone()), ciphertext)?;

        // The fetcher must resolve chunk_id → blob_id via the snapshot metadata map.
        let mut blobs_map = HashMap::new();
        blobs_map.insert(
            chunk_id_hex.clone(),
            AppBlobMeta {
                id: hex::decode(&blob_id_hex)?,
                length: plaintext.len() as u32,
                ..Default::default()
            },
        );

        let fetcher = AppChunkFetcher::new(repo_path, key, Arc::new(blobs_map))?;

        let chunk_id: AppChunkId = chunk_id_hex.parse()?;
        let fetched_data = fetcher.fetch_chunk(chunk_id)?;

        assert_eq!(fetched_data, plaintext);

        Ok(())
    }
}
