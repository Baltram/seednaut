use crate::engine::crypto::{create_streaming_aead, derive_keys};
use crate::engine::scanner::discover_snapshots;
use crate::engine::types::pb::seedvault::Snapshot;
use anyhow::Result;
use bip39::{Language, Mnemonic};
use prost::Message;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tink_core::StreamingAead;

const APP_BACKUP_VERSION: u8 = 2;

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

fn encrypt_app_snapshot(
    snapshot: &Snapshot,
    aead: &dyn StreamingAead,
    out_path: &Path,
) -> Result<()> {
    let mut buf = Vec::new();
    snapshot.encode(&mut buf)?;

    let compressed = zstd::encode_all(buf.as_slice(), 0)?;

    // Encrypted file format: [version_byte][AEAD_encrypted(
    //   compressed_size:4BE || compressed_data
    // )]
    // AD for app snapshots is just the version byte.
    let ad = vec![APP_BACKUP_VERSION];
    let buffer = Arc::new(Mutex::new(Vec::new()));
    buffer.lock().unwrap().push(APP_BACKUP_VERSION);

    let writer_buf = SharedBuffer {
        data: buffer.clone(),
    };
    let mut writer = aead
        .new_encrypting_writer(Box::new(writer_buf), &ad)
        .map_err(|e| anyhow::anyhow!("Encryption init failed: {:?}", e))?;

    let size_bytes = (compressed.len() as u32).to_be_bytes();
    writer.write_all(&size_bytes)?;
    writer.write_all(&compressed)?;
    writer.flush()?;
    drop(writer);

    let ciphertext = Arc::try_unwrap(buffer).unwrap().into_inner().unwrap();

    // App snapshots are stored as `<64-char-hex-repo>/<sha256-hash>.snapshot`
    let hash = hex::encode(Sha256::digest(&ciphertext));

    let parent_hash = "0000000000000000000000000000000000000000000000000000000000000000";
    let parent_dir = out_path.join(parent_hash);
    fs::create_dir_all(&parent_dir)?;

    let file_path = parent_dir.join(format!("{}.snapshot", hash));
    fs::write(file_path, ciphertext)?;

    Ok(())
}

#[test]
fn test_discover_app_snapshot() -> Result<()> {
    let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let mnemonic = Mnemonic::parse_in(Language::English, phrase)?;
    let derived_keys = derive_keys(&mnemonic)?;
    let app_aead = create_streaming_aead(derived_keys.app_stream_key)?;

    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();

    let snapshot = Snapshot {
        version: 1,
        name: "Test App".to_string(),
        token: 1234567890,
        ..Default::default()
    };

    encrypt_app_snapshot(&snapshot, &*app_aead, root)?;

    let snapshots = discover_snapshots(root, &derived_keys)?;
    assert!(snapshots.failed.is_empty());

    assert_eq!(snapshots.snapshots.len(), 1);
    let s = &snapshots.snapshots[0];
    assert_eq!(s.name, "Test App");
    assert_eq!(s.timestamp, 1234567890);

    Ok(())
}

const FILE_BACKUP_VERSION: u8 = 0;

fn encrypt_file_snapshot(
    snapshot: &crate::engine::types::pb::calyxos::BackupSnapshot,
    aead: &dyn StreamingAead,
    out_path: &Path,
) -> Result<()> {
    let mut buf = Vec::new();
    snapshot.encode(&mut buf)?;

    // AD for file snapshot: [version, type=0x01, timestamp:8BE]
    let mut ad = vec![FILE_BACKUP_VERSION, 0x01];
    ad.extend_from_slice(&snapshot.time_start.to_be_bytes());

    let buffer = Arc::new(Mutex::new(Vec::new()));
    buffer.lock().unwrap().push(FILE_BACKUP_VERSION);

    let writer_buf = SharedBuffer {
        data: buffer.clone(),
    };
    let mut writer = aead
        .new_encrypting_writer(Box::new(writer_buf), &ad)
        .map_err(|e| anyhow::anyhow!("Encryption init failed: {:?}", e))?;

    // File snapshots are not compressed.
    writer.write_all(&buf)?;
    writer.flush()?;
    drop(writer);

    let ciphertext = Arc::try_unwrap(buffer).unwrap().into_inner().unwrap();

    // File snapshots are stored as `<16-char-hex>.sv/<timestamp>.SeedSnap`
    let parent_hash = "0000000000000000.sv";
    let parent_dir = out_path.join(parent_hash);
    fs::create_dir_all(&parent_dir)?;

    let file_path = parent_dir.join(format!("{}.SeedSnap", snapshot.time_start));
    fs::write(file_path, ciphertext)?;

    Ok(())
}

#[test]
fn test_discover_file_snapshot() -> Result<()> {
    let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
    let mnemonic = Mnemonic::parse_in(Language::English, phrase)?;
    let derived_keys = derive_keys(&mnemonic)?;
    let file_aead = create_streaming_aead(derived_keys.file_stream_key)?;

    let temp_dir = TempDir::new()?;
    let root = temp_dir.path();

    let snapshot = crate::engine::types::pb::calyxos::BackupSnapshot {
        version: 1,
        time_start: 1600000000000,
        ..Default::default()
    };

    encrypt_file_snapshot(&snapshot, &*file_aead, root)?;

    let snapshots = discover_snapshots(root, &derived_keys)?;
    assert!(snapshots.failed.is_empty());

    assert_eq!(snapshots.snapshots.len(), 1);
    let s = &snapshots.snapshots[0];
    assert_eq!(s.timestamp, 1600000000000);
    assert!(matches!(
        s.raw_snapshot,
        crate::engine::types::RawSnapshot::File(_)
    ));

    Ok(())
}
