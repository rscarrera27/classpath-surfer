use std::collections::HashSet;

use super::ClasspathManifest;

/// GAV-level diff between two classpath manifests, used for incremental indexing.
///
/// Each set contains GAV strings (`group:artifact:version`).
pub struct ManifestDiff {
    /// GAVs present in the current manifest but absent from the previous one.
    pub added: HashSet<String>,
    /// GAVs present in the previous manifest but absent from the current one.
    pub removed: HashSet<String>,
    /// GAVs present in both manifests (no re-indexing needed).
    pub unchanged: HashSet<String>,
}

/// Compute the GAV-level diff between a `current` and `previous` manifest.
///
/// The diff drives incremental indexing: added GAVs are indexed, removed GAVs
/// are deleted from the index, and unchanged GAVs are left as-is.
pub fn compute_diff(current: &ClasspathManifest, previous: &ClasspathManifest) -> ManifestDiff {
    let current_gavs: HashSet<String> =
        current.all_dependencies().iter().map(|d| d.gav()).collect();
    let previous_gavs: HashSet<String> = previous
        .all_dependencies()
        .iter()
        .map(|d| d.gav())
        .collect();

    ManifestDiff {
        added: current_gavs.difference(&previous_gavs).cloned().collect(),
        removed: previous_gavs.difference(&current_gavs).cloned().collect(),
        unchanged: current_gavs.intersection(&previous_gavs).cloned().collect(),
    }
}
