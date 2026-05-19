pub mod seedvault {
    #![allow(clippy::nursery, clippy::pedantic)]
    include!("generated/com.stevesoltys.seedvault.proto.rs");
}
pub mod calyxos {
    #![allow(clippy::nursery, clippy::pedantic)]
    include!("generated/org.calyxos.backup.storage.backup.rs");
}

pub const DESCRIPTOR_BYTES: &[u8] = include_bytes!("generated/descriptor.bin");
