fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = prost_build::Config::new();

    // Type message has recursive fields — must be boxed
    config.boxed("Type.flexible_upper_bound");
    config.boxed("Type.outer_type");
    config.boxed("Type.abbreviated_type");

    // #![deny(missing_docs)] — generated code has no doc comments
    config.type_attribute(".", "#[allow(missing_docs)]");

    config.compile_protos(&["proto/kotlin_metadata.proto"], &["proto/"])?;

    Ok(())
}
