use std::process::Command;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = prost_build::Config::new();

    // Type message has recursive fields — must be boxed
    config.boxed("Type.flexible_upper_bound");
    config.boxed("Type.outer_type");
    config.boxed("Type.abbreviated_type");

    // #![deny(missing_docs)] — generated code has no doc comments
    config.type_attribute(".", "#[allow(missing_docs)]");

    config.compile_protos(&["proto/kotlin_metadata.proto"], &["proto/"])?;

    // Expose git SHA for --version long display
    let git_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_SHA={git_hash}");

    // Only re-run when git HEAD changes (not on every build)
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/");

    Ok(())
}
