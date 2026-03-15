//! Source code resolution for dependency symbols.
//!
//! Attempts to locate the original source from a source JAR first,
//! falling back to decompilation (e.g. CFR) when sources are unavailable.

/// External decompiler integration (CFR, Vineflower).
pub mod decompiler;
/// Method line-number resolution via classfile `LineNumberTable`.
pub mod locator;
/// Source code lookup with source-JAR → decompilation fallback.
pub mod resolver;
