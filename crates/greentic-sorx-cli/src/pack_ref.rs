//! Where a `start` pack comes from: a local file, or an OCI reference pulled
//! at boot (so a container needs no volume for its pack).

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // used by `materialize` (next task)
pub(crate) enum PackRef {
    Local(PathBuf),
    /// Registry reference without the `oci://` scheme.
    Oci(String),
}

#[allow(dead_code)] // used by `materialize` (next task)
pub(crate) fn parse_pack_ref(raw: &Path) -> PackRef {
    match raw.to_str().and_then(|text| text.strip_prefix("oci://")) {
        Some(rest) if !rest.trim().is_empty() => PackRef::Oci(rest.trim().to_string()),
        _ => PackRef::Local(raw.to_path_buf()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn a_plain_path_stays_local() {
        assert_eq!(
            parse_pack_ref(Path::new("landlord.gtpack")),
            PackRef::Local(PathBuf::from("landlord.gtpack"))
        );
    }

    #[test]
    fn an_oci_reference_drops_its_scheme() {
        assert_eq!(
            parse_pack_ref(Path::new("oci://reg.example/greentic/sor:t1@sha256:ab")),
            PackRef::Oci("reg.example/greentic/sor:t1@sha256:ab".to_string())
        );
    }

    #[test]
    fn a_bare_scheme_with_nothing_after_it_is_not_a_reference() {
        assert_eq!(
            parse_pack_ref(Path::new("oci://")),
            PackRef::Local(PathBuf::from("oci://"))
        );
    }
}
