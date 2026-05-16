use anyhow::Result;
use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

/// Represents a 32-byte hash-based ID used for different purposes depending on the context.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Id([u8; 32]);

/// SHA256 hash of an uncompressed, decrypted app data chunk.
/// Used as a key in the app snapshot's `blobs` map.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AppChunkId(pub Id);

/// SHA256 hash of an encrypted, compressed blob file on disk (its filename).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlobId(pub Id);

/// HMAC-SHA256 hash of an uncompressed file backup chunk (its filename).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileChunkId(pub Id);

impl Id {
    /// Returns the raw bytes of the ID.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl<'a> TryFrom<&'a [u8]> for Id {
    type Error = std::array::TryFromSliceError;

    fn try_from(bytes: &'a [u8]) -> Result<Self, Self::Error> {
        Ok(Self(bytes.try_into()?))
    }
}

impl FromStr for Id {
    type Err = hex::FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(s, &mut bytes)?;
        Ok(Self(bytes))
    }
}

impl From<[u8; 32]> for Id {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

impl fmt::Debug for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Id({})", self)
    }
}

macro_rules! impl_id_traits {
    ($specific_id:ty, $name:literal) => {
        impl From<Id> for $specific_id {
            fn from(id: Id) -> Self {
                Self(id)
            }
        }

        impl From<$specific_id> for Id {
            fn from(id: $specific_id) -> Self {
                id.0
            }
        }

        impl fmt::Display for $specific_id {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl fmt::Debug for $specific_id {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", $name, self)
            }
        }

        impl FromStr for $specific_id {
            type Err = hex::FromHexError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Id::from_str(s).map(Self)
            }
        }

        impl<'a> TryFrom<&'a [u8]> for $specific_id {
            type Error = std::array::TryFromSliceError;

            fn try_from(bytes: &'a [u8]) -> Result<Self, Self::Error> {
                Id::try_from(bytes).map(Self::from)
            }
        }
    };
}

impl_id_traits!(AppChunkId, "AppChunkId");
impl_id_traits!(BlobId, "BlobId");
impl_id_traits!(FileChunkId, "FileChunkId");

/// Contains all prost-generated structs from the .proto files.
pub mod pb {
    pub mod seedvault {
        #![allow(clippy::nursery, clippy::pedantic)]
        include!(concat!(
            env!("OUT_DIR"),
            "/com.stevesoltys.seedvault.proto.rs"
        ));
    }
    pub mod calyxos {
        #![allow(clippy::nursery, clippy::pedantic)]
        include!(concat!(
            env!("OUT_DIR"),
            "/org.calyxos.backup.storage.backup.rs"
        ));
    }
}

/// An enum representing the two types of snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotType {
    App,
    File,
}

/// A unified representation of a parsed snapshot, whether it's for apps or files.
#[derive(Debug)]
pub enum RawSnapshot {
    App(pb::seedvault::Snapshot),
    File(pb::calyxos::BackupSnapshot),
}

/// High-level information about a discovered and successfully decrypted snapshot.
#[derive(Debug)]
pub struct SnapshotInfo {
    pub index: u32,
    /// Milliseconds since epoch. For app snapshots this is `token`; for file snapshots it is `time_start`.
    pub timestamp: u64,
    pub name: String,
    pub snapshot_path: PathBuf,
    pub repo_path: PathBuf,
    pub raw_snapshot: RawSnapshot,
}

impl SnapshotInfo {
    /// Returns the type of this snapshot.
    pub fn snapshot_type(&self) -> SnapshotType {
        match self.raw_snapshot {
            RawSnapshot::App(_) => SnapshotType::App,
            RawSnapshot::File(_) => SnapshotType::File,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_id_from_str() {
        let hex = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
        let id: Id = hex.parse().unwrap();
        assert_eq!(id.as_bytes()[0], 0x00);
        assert_eq!(id.as_bytes()[31], 0x1f);
    }

    #[test]
    fn test_id_display() {
        let bytes = [0xabu8; 32];
        let id = Id::from(bytes);
        let s = id.to_string();
        assert_eq!(
            s,
            "abababababababababababababababababababababababababababababababab"
        );
    }

    #[test]
    fn test_specific_id_traits() {
        let hex = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
        let id: AppChunkId = hex.parse().unwrap();
        assert_eq!(id.to_string(), hex);
    }
}
