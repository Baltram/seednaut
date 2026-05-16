fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=proto/snapshot.proto");
    println!("cargo:rerun-if-changed=proto/backup_snapshot.proto");
    println!("cargo:rerun-if-changed=proto/backup_media_file.proto");
    println!("cargo:rerun-if-changed=proto/backup_document_file.proto");

    let mut config = prost_build::Config::new();

    // Generate a file descriptor set so prost-reflect can dynamically
    // deserialize protobuf messages to JSON at runtime.
    let out_dir = std::env::var("OUT_DIR")?;
    let descriptor_path = std::path::PathBuf::from(out_dir).join("descriptor.bin");

    config.file_descriptor_set_path(&descriptor_path);

    config.compile_protos(
        &[
            "proto/snapshot.proto",
            "proto/backup_snapshot.proto",
            "proto/backup_media_file.proto",
            "proto/backup_document_file.proto",
        ],
        &["proto/"],
    )?;

    Ok(())
}
