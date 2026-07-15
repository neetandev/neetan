//! Stable logical identities for mounted media sources.

use std::sync::atomic::{AtomicU64, Ordering};

use save_state::{FingerprintBuilder, MediaSourcePath, ResourceIdentity, encode_runtime_state};

static NEXT_ANONYMOUS_IDENTITY: AtomicU64 = AtomicU64::new(1);

pub(crate) fn path_identity(
    domain: &str,
    source_path: &MediaSourcePath,
    byte_length: u64,
    structure: &[u8],
) -> ResourceIdentity {
    let mut builder = FingerprintBuilder::new(domain);
    builder.add("source-path", &encode_runtime_state(source_path));
    builder.add("structure", structure);
    ResourceIdentity::new(builder.finish(), byte_length)
}

pub(crate) fn anonymous_identity(
    domain: &str,
    byte_length: u64,
    structure: &[u8],
) -> ResourceIdentity {
    let identifier = NEXT_ANONYMOUS_IDENTITY.fetch_add(1, Ordering::Relaxed);
    let mut builder = FingerprintBuilder::new(domain);
    builder.add("anonymous", &identifier.to_le_bytes());
    builder.add("structure", structure);
    ResourceIdentity::new(builder.finish(), byte_length)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn identity(path: &str) -> ResourceIdentity {
        path_identity(
            "test-media",
            &MediaSourcePath::from_path(Path::new(path)),
            123,
            b"layout",
        )
    }

    #[test]
    fn equivalent_relative_paths_match() {
        assert_eq!(identity("media/disc.cue"), identity("./media/disc.cue"));
        assert_eq!(
            identity("games/../media/disc.cue"),
            identity("media/disc.cue")
        );
    }

    #[test]
    fn leading_parents_and_roots_remain_distinct() {
        assert_ne!(identity("../media/disc.cue"), identity("media/disc.cue"));
        assert_ne!(identity("/media/disc.cue"), identity("media/disc.cue"));
    }

    #[test]
    fn anonymous_sources_are_unique() {
        assert_ne!(
            anonymous_identity("test-media", 123, b"layout"),
            anonymous_identity("test-media", 123, b"layout")
        );
    }
}
