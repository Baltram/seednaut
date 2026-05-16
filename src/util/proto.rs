use anyhow::{Context, Result};
use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage};
use std::sync::OnceLock;

/// Descriptor pool loaded from the generated file descriptor set
static DESCRIPTOR_POOL: OnceLock<DescriptorPool> = OnceLock::new();

/// Get or initialize the descriptor pool
fn get_descriptor_pool() -> &'static DescriptorPool {
    DESCRIPTOR_POOL.get_or_init(|| {
        let descriptor_bytes = include_bytes!(concat!(env!("OUT_DIR"), "/descriptor.bin"));
        DescriptorPool::decode(descriptor_bytes.as_ref()).expect("Failed to decode descriptor pool")
    })
}

/// Serialize an app Snapshot protobuf message to properly formatted JSON
pub fn serialize_app_snapshot(
    snapshot: &crate::engine::types::pb::seedvault::Snapshot,
) -> Result<String> {
    to_json_pretty_hex_bytes(
        "com.stevesoltys.seedvault.proto.Snapshot",
        &snapshot.encode_to_vec(),
    )
}

/// Serialize a file BackupSnapshot protobuf message to properly formatted JSON
pub fn serialize_file_snapshot(
    snapshot: &crate::engine::types::pb::calyxos::BackupSnapshot,
) -> Result<String> {
    to_json_pretty_hex_bytes(
        "org.calyxos.backup.storage.backup.BackupSnapshot",
        &snapshot.encode_to_vec(),
    )
}

/// Serialize a protobuf message to JSON with hex-encoded bytes instead of base64.
/// This is useful for chunk IDs which are more readable in hex format.
fn to_json_pretty_hex_bytes(message_name: &str, encoded_bytes: &[u8]) -> Result<String> {
    let pool = get_descriptor_pool();

    let message_descriptor = pool.get_message_by_name(message_name).with_context(|| {
        format!(
            "Message type '{}' not found in descriptor pool",
            message_name
        )
    })?;

    let dynamic = DynamicMessage::decode(message_descriptor, encoded_bytes)
        .context("Failed to decode message bytes")?;

    // prost-reflect serializes bytes as base64; convert 32-byte IDs to hex.
    let mut value: serde_json::Value =
        serde_json::to_value(&dynamic).context("Failed to serialize to JSON value")?;

    convert_base64_to_hex(&mut value);

    serde_json::to_string_pretty(&value).context("Failed to serialize to JSON")
}

/// Recursively convert 32-byte base64-encoded values to hex strings.
fn convert_base64_to_hex(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(s) => {
            // Try to decode as base64, if successful and looks like it could be a chunk ID or hash, convert to hex
            if let Ok(bytes) =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, s.as_str())
                && !bytes.is_empty()
                && bytes.len() == 32
            {
                *s = hex::encode(&bytes);
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                convert_base64_to_hex(item);
            }
        }
        serde_json::Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                convert_base64_to_hex(v);
            }
        }
        _ => {}
    }
}
