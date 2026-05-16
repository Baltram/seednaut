use anyhow::{Context, Result, anyhow, ensure};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Cursor, Read};
use std::path::Path;

const APP_BACKUP_VERSION: u8 = 2;
const FILE_BACKUP_VERSION: u8 = 0;

/// Specifies the type of a repository file to guide the decryption process.
/// This determines the expected version, associated data, and whether decompression is needed.
#[derive(Debug, Clone, Copy)]
pub enum RepoFileType<'a> {
    /// An app backup snapshot file (`.snapshot`).
    AppSnapshot,
    /// A chunk belonging to an app backup.
    AppChunk,
    /// A file backup snapshot file (`.SeedSnap`).
    FileSnapshot { timestamp: u64 },
    /// A chunk belonging to a file backup.
    FileChunk { chunk_id: &'a [u8] },
}

impl RepoFileType<'_> {
    /// Returns true if the file type is expected to be zstd-compressed after decryption.
    fn is_compressed(&self) -> bool {
        matches!(self, Self::AppSnapshot | Self::AppChunk)
    }

    /// Returns true if the filename should be verified against the SHA256 hash of its content.
    fn needs_filename_verification(&self) -> bool {
        matches!(self, Self::AppSnapshot | Self::AppChunk)
    }
}

/// Decrypts, verifies integrity, and decompresses (if compressed) a repository file.
pub fn decrypt_and_decompress(
    path: &Path,
    aead: &dyn tink_core::StreamingAead,
    file_type: RepoFileType,
) -> Result<Vec<u8>> {
    let encrypted_bytes = fs::read(path)
        .with_context(|| format!("Failed to read repository file: {}", path.display()))?;

    // For app data, the filename (sans extension) is the SHA256 hash of the encrypted content.
    if file_type.needs_filename_verification() {
        let expected_hash = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_lowercase();

        let actual_hash = hex::encode(Sha256::digest(&encrypted_bytes));

        ensure!(
            actual_hash == expected_hash,
            "File hash mismatch for {}: expected {}, got {}",
            path.display(),
            expected_hash,
            actual_hash
        );
    }

    // The first byte is the version, the rest is ciphertext.
    let (version, ciphertext) = encrypted_bytes
        .split_first()
        .ok_or_else(|| anyhow!("File is empty: {}", path.display()))?;

    // Construct associated data. The structure depends on the file type.
    let associated_data = {
        let mut ad = vec![*version];
        match file_type {
            RepoFileType::AppSnapshot | RepoFileType::AppChunk => {
                ensure!(
                    *version == APP_BACKUP_VERSION,
                    "Unsupported app backup version {}. Only version {} is supported.",
                    version,
                    APP_BACKUP_VERSION
                );
            }
            RepoFileType::FileSnapshot { timestamp } => {
                ensure!(
                    *version == FILE_BACKUP_VERSION,
                    "Unsupported file snapshot version {}. Only version {} is supported.",
                    version,
                    FILE_BACKUP_VERSION
                );
                // Type byte: 0x01 for snapshots, 0x00 for chunks.
                ad.push(0x01);
                ad.extend_from_slice(&timestamp.to_be_bytes());
            }
            RepoFileType::FileChunk { chunk_id } => {
                ensure!(
                    *version == FILE_BACKUP_VERSION,
                    "Unsupported file chunk version {}. Only version {} is supported.",
                    version,
                    FILE_BACKUP_VERSION
                );
                ad.push(0x00);
                ad.extend_from_slice(chunk_id);
            }
        }
        ad
    };

    // Decrypt on the fly.
    let mut decrypting_reader = aead
        .new_decrypting_reader(Box::new(Cursor::new(ciphertext.to_vec())), &associated_data)
        .map_err(|e| {
            anyhow!(
                "Decryption failed for {}. Wrong key or corrupted data: {}",
                path.display(),
                e
            )
        })?;

    if file_type.is_compressed() {
        // App data is compressed. Read the 4-byte big-endian compressed payload size.
        let mut size_buf = [0u8; 4];
        decrypting_reader
            .read_exact(&mut size_buf)
            .context("Could not read compressed payload size")?;
        let payload_size = u32::from_be_bytes(size_buf) as usize;

        // Sanity limit to prevent excessive memory allocation.
        ensure!(
            payload_size <= 64 * 1024 * 1024,
            "Decrypted payload size {} exceeds sanity limit",
            payload_size
        );

        // Read compressed payload and decompress with zstd.
        let mut compressed_payload = vec![0u8; payload_size];
        decrypting_reader
            .read_exact(&mut compressed_payload)
            .context("Failed to read full compressed payload")?;

        // Snapshots have no padding, but blobs/chunks are always padded
        // (Padme algorithm). The padding must be discarded.
        if matches!(file_type, RepoFileType::AppSnapshot) {
            let mut extra = [0u8; 1];
            ensure!(
                decrypting_reader.read(&mut extra)? == 0,
                "Trailing decrypted data after compressed payload"
            );
        }

        // Decompress the payload using zstd.
        zstd::decode_all(Cursor::new(compressed_payload)).context("Zstd decompression failed")
    } else {
        // File data is not compressed. Read the entire decrypted stream.
        let mut decrypted_data = Vec::new();
        decrypting_reader
            .read_to_end(&mut decrypted_data)
            .context("Failed to read decrypted data")?;
        Ok(decrypted_data)
    }
}
