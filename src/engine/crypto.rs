use anyhow::{Context, Result, anyhow};
use bip39::Mnemonic;
use hkdf::Hkdf;
use prost::Message;
use sha2::Sha256;

/// The BIP39 passphrase used for seed derivation. Seedvault uses an empty passphrase.
const BIP39_PASSPHRASE: &str = "";

/// A 32-byte key used for various cryptographic operations.
pub type Key = [u8; 32];

/// A collection of all keys derived from the user's mnemonic phrase.
pub struct DerivedKeys {
    pub app_stream_key: Key,
    pub file_stream_key: Key,
    pub chunk_id_key: Key,
}

/// Derives all necessary keys from a BIP39 mnemonic phrase.
///
/// The derivation follows the process used by Seedvault:
/// 1. A BIP39 seed is generated from the mnemonic.
/// 2. The `main_key` is the second 32-byte chunk of this seed.
/// 3. All other keys are derived from the `main_key` using HKDF-SHA256 with specific "info" strings.
pub fn derive_keys(mnemonic: &Mnemonic) -> Result<DerivedKeys> {
    // BIP39 seed generation using the Seedvault standard passphrase.
    let seed = mnemonic.to_seed(BIP39_PASSPHRASE);

    let main_key: Key = seed[32..64]
        .try_into()
        .context("Failed to extract main_key from seed")?;

    let hk = Hkdf::<Sha256>::from_prk(&main_key)
        .map_err(|e| anyhow!("Invalid PRK length for HKDF: {}", e))?;

    let expand_key = |info: &[u8]| -> Result<Key> {
        let mut okm = [0u8; 32];
        hk.expand(info, &mut okm).map_err(|e| {
            anyhow!(
                "HKDF expand failed for info '{:?}': {}",
                String::from_utf8_lossy(info),
                e
            )
        })?;
        Ok(okm)
    };

    Ok(DerivedKeys {
        app_stream_key: expand_key(b"app backup stream key")?,
        file_stream_key: expand_key(b"stream key")?,
        chunk_id_key: expand_key(b"Chunk ID calculation")?,
    })
}

/// Creates a Tink StreamingAead primitive from a raw 32-byte key.
///
/// This constructs a Tink Keyset in-memory containing a single
/// AES256-GCM-HKDF key with a 1MB segment size and a RAW output prefix.
pub fn create_streaming_aead(key: Key) -> Result<Box<dyn tink_core::StreamingAead>> {
    tink_streaming_aead::init();

    let key_template = tink_streaming_aead::aes256_gcm_hkdf_1mb_key_template();
    let key_format = tink_proto::AesGcmHkdfStreamingKeyFormat::decode(key_template.value.as_ref())
        .context("Failed to decode key format from template")?;

    let streaming_key_proto = tink_proto::AesGcmHkdfStreamingKey {
        version: key_format.version,
        params: key_format.params,
        key_value: key.to_vec(),
    };
    let key_data = tink_proto::KeyData {
        type_url: key_template.type_url.clone(),
        value: streaming_key_proto.encode_to_vec(),
        key_material_type: tink_proto::key_data::KeyMaterialType::Symmetric as i32,
    };
    let keyset_key = tink_proto::keyset::Key {
        key_data: Some(key_data),
        status: tink_proto::KeyStatusType::Enabled as i32,
        key_id: 1,
        output_prefix_type: tink_proto::OutputPrefixType::Raw as i32,
    };
    let keyset = tink_proto::Keyset {
        primary_key_id: 1,
        key: vec![keyset_key],
    };

    // Parse the Keyset proto into a KeysetHandle. This uses `insecure::new_handle` because we are providing
    // the raw key material directly, which is the intended use case here.
    let handle = tink_core::keyset::insecure::new_handle(keyset)
        .map_err(|e| anyhow!("Failed to create Tink KeysetHandle: {}", e))?;

    let primitive = tink_streaming_aead::new(&handle)
        .map_err(|e| anyhow!("Failed to create StreamingAead primitive: {}", e))?;
    Ok(primitive)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bip39::{Language, Mnemonic};
    use std::io::{Cursor, Read, Write};
    use std::sync::{Arc, Mutex};

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
    fn test_derive_keys_deterministic() {
        // Use a fixed mnemonic for reproducibility
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let mnemonic = Mnemonic::parse_in(Language::English, phrase).unwrap();

        let keys1 = derive_keys(&mnemonic).unwrap();
        let keys2 = derive_keys(&mnemonic).unwrap();

        assert_eq!(keys1.app_stream_key, keys2.app_stream_key);
        assert_eq!(keys1.file_stream_key, keys2.file_stream_key);
        assert_eq!(keys1.chunk_id_key, keys2.chunk_id_key);
    }

    #[test]
    fn test_create_streaming_aead_encrypt_decrypt() {
        let key = [42u8; 32];
        let aead = create_streaming_aead(key).unwrap();

        let plaintext = b"Hello, Seednaut!";
        let aad = b"associated data";

        // Encrypt
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let writer_buf = SharedBuffer {
            data: buffer.clone(),
        };

        let mut writer = aead
            .new_encrypting_writer(Box::new(writer_buf), aad)
            .map_err(|e| anyhow!("Encryption failed: {:?}", e))
            .unwrap();
        writer.write_all(plaintext).unwrap();
        writer.flush().unwrap();
        // Explicitly drop writer to finish the stream
        drop(writer);

        let ciphertext = Arc::try_unwrap(buffer).unwrap().into_inner().unwrap();
        assert!(!ciphertext.is_empty());

        // Decrypt
        let mut decrypted = Vec::new();
        let mut reader = aead
            .new_decrypting_reader(Box::new(Cursor::new(ciphertext)), aad)
            .map_err(|e| anyhow!("Decryption failed: {:?}", e))
            .unwrap();
        reader.read_to_end(&mut decrypted).unwrap();

        assert_eq!(decrypted, plaintext);
    }
}
