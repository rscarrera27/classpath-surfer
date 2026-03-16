//! Gradle init script generation and execution.
//!
//! Generates a Gradle init script that resolves all library JARs and
//! their source JARs, then invokes `gradle` (or `gradlew`) to run it.

/// Embedded Gradle Groovy init script for classpath export.
pub mod init_script;
/// Gradle process launcher.
pub mod runner;
