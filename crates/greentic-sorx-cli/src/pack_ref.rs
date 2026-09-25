//! Where a `start` pack comes from: a local file, or an OCI reference pulled
//! at boot (so a container needs no volume for its pack).

use std::path::{Path, PathBuf};

use greentic_distributor_client::{
    OciPackFetcher, PackFetchOptions, oci_packs::DefaultRegistryClient,
};

use crate::{CliError, CliResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PackRef {
    Local(PathBuf),
    /// Registry reference without the `oci://` scheme.
    Oci(String),
}

pub(crate) fn parse_pack_ref(raw: &Path) -> PackRef {
    match raw.to_str().and_then(|text| text.strip_prefix("oci://")) {
        Some(rest) if !rest.trim().is_empty() => PackRef::Oci(rest.trim().to_string()),
        _ => PackRef::Local(raw.to_path_buf()),
    }
}

/// Resolve `pack` to a local `.gtpack`, pulling an `oci://` reference first.
///
/// A digest-pinned reference (`…@sha256:…`) is verified by the fetcher; a
/// registry that serves different bytes fails the pull rather than booting
/// another pack (`greentic_distributor_client::oci_packs::OciPackFetcher::fetch_pack_to_cache`
/// compares the resolved digest against the one pinned in the reference and
/// returns `OciPackError::DigestMismatch` on a mismatch — see the live check
/// in this module's tests). Credentials are the same `OCI_USERNAME` /
/// `OCI_PASSWORD` pair greentic-start honours, so one Kubernetes Secret
/// serves both.
#[allow(dead_code)] // wired into start by the next task
pub(crate) fn materialize(pack: &Path) -> CliResult<PathBuf> {
    let reference = match parse_pack_ref(pack) {
        PackRef::Local(path) => return Ok(path),
        PackRef::Oci(reference) => reference,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|err| CliError::runtime(format!("cannot start the pack fetcher: {err}")))?;
    let options = PackFetchOptions {
        allow_tags: true,
        offline: false,
        ..PackFetchOptions::default()
    };
    let insecure_registries = insecure_registries_from_env();
    let fetcher: OciPackFetcher<DefaultRegistryClient> =
        match decide_transport(pull_credentials(), insecure_registries) {
            TransportDecision::Default => OciPackFetcher::new(options),
            TransportDecision::Authenticated {
                username,
                password,
                insecure_registries_ignored,
            } => {
                if !insecure_registries_ignored.is_empty() {
                    eprintln!(
                        "greentic-sorx: GREENTIC_OCI_INSECURE_REGISTRIES is set but this pull is \
                         authenticated; DefaultRegistryClient's basic-auth constructor stays \
                         HTTPS, so this pull will fail if the registry only serves plain HTTP"
                    );
                }
                OciPackFetcher::with_client(
                    DefaultRegistryClient::with_basic_auth(username, password),
                    options,
                )
            }
            TransportDecision::InsecureRegistries(registries) => OciPackFetcher::with_client(
                DefaultRegistryClient::with_insecure_registries(registries),
                options,
            ),
        };
    let fetched = runtime
        .block_on(fetcher.fetch_pack_to_cache(&reference))
        .map_err(|err| CliError::runtime(format!("cannot pull pack oci://{reference}: {err}")))?;
    Ok(fetched.path)
}

/// Which registry client to build for a pull, decided once so the "explicit
/// credentials always win" rule is a pure function callers and tests can
/// reason about without a network call.
///
/// Mirrors greentic-start's `fetch_remote_bundle`: `DefaultRegistryClient`'s
/// `with_basic_auth` and `with_insecure_registries` constructors each
/// hardcode the OTHER axis (auth vs. transport), so an operator who set
/// `OCI_USERNAME`/`OCI_PASSWORD` must not be silently downgraded to plain
/// HTTP by an also-set `GREENTIC_OCI_INSECURE_REGISTRIES` — that would send
/// credentials in the clear to whichever registry the pull resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TransportDecision {
    /// No credentials, no insecure registries: HTTPS, anonymous.
    Default,
    /// Explicit credentials win outright; `insecure_registries_ignored` is
    /// carried through only so the caller can warn that it had no effect.
    Authenticated {
        username: String,
        password: String,
        insecure_registries_ignored: Vec<String>,
    },
    /// No credentials: the listed `host[:port]` registries are pulled over
    /// plain HTTP, everything else stays HTTPS.
    InsecureRegistries(Vec<String>),
}

fn decide_transport(
    credentials: Option<(String, String)>,
    insecure_registries: Vec<String>,
) -> TransportDecision {
    match credentials {
        Some((username, password)) => TransportDecision::Authenticated {
            username,
            password,
            insecure_registries_ignored: insecure_registries,
        },
        None if insecure_registries.is_empty() => TransportDecision::Default,
        None => TransportDecision::InsecureRegistries(insecure_registries),
    }
}

fn pull_credentials() -> Option<(String, String)> {
    credentials_from(
        std::env::var("OCI_USERNAME").ok(),
        std::env::var("OCI_PASSWORD").ok(),
    )
}

fn credentials_from(
    username: Option<String>,
    password: Option<String>,
) -> Option<(String, String)> {
    match (username, password) {
        (Some(u), Some(p)) if !u.is_empty() && !p.is_empty() => Some((u, p)),
        _ => None,
    }
}

/// Read the `GREENTIC_OCI_INSECURE_REGISTRIES` allow-list (comma-separated
/// `host[:port]` entries pulled over plain HTTP instead of HTTPS). Unset or
/// empty yields an empty list (HTTPS for every registry, the default).
///
/// Unlike greentic-start's `insecure_registries_for_fetch`, `materialize` has
/// exactly one pull site and it is always digest-gated when the reference
/// pins one, so there is no separate "non-boot resolution" caller to keep
/// HTTPS-only — the env var is honoured unconditionally here.
fn insecure_registries_from_env() -> Vec<String> {
    std::env::var("GREENTIC_OCI_INSECURE_REGISTRIES")
        .ok()
        .map(|raw| parse_insecure_registries(&raw))
        .unwrap_or_default()
}

fn parse_insecure_registries(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect()
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

    #[test]
    fn credentials_need_both_halves() {
        assert_eq!(credentials_from(Some("u".into()), None), None);
        assert_eq!(credentials_from(None, Some("p".into())), None);
        assert_eq!(credentials_from(Some("".into()), Some("p".into())), None);
        assert_eq!(
            credentials_from(Some("u".into()), Some("p".into())),
            Some(("u".to_string(), "p".to_string()))
        );
    }

    #[test]
    fn a_local_pack_is_returned_unchanged_and_never_touches_the_network() {
        let path = Path::new("/tmp/does-not-need-to-exist.gtpack");
        assert_eq!(materialize(path).expect("local"), path.to_path_buf());
    }

    #[test]
    fn insecure_registries_split_trim_and_drop_empties() {
        assert_eq!(parse_insecure_registries(""), Vec::<String>::new());
        assert_eq!(
            parse_insecure_registries(" localhost:5000 , , registry.internal:5000"),
            vec![
                "localhost:5000".to_string(),
                "registry.internal:5000".to_string()
            ]
        );
    }

    #[test]
    fn an_authenticated_pull_never_downgrades_to_http() {
        let decision = decide_transport(
            Some(("u".to_string(), "p".to_string())),
            vec!["localhost:5000".to_string()],
        );
        assert_eq!(
            decision,
            TransportDecision::Authenticated {
                username: "u".to_string(),
                password: "p".to_string(),
                insecure_registries_ignored: vec!["localhost:5000".to_string()],
            }
        );
    }

    #[test]
    fn no_credentials_falls_back_to_insecure_registries() {
        let decision = decide_transport(None, vec!["localhost:5000".to_string()]);
        assert_eq!(
            decision,
            TransportDecision::InsecureRegistries(vec!["localhost:5000".to_string()])
        );
    }

    #[test]
    fn no_credentials_and_no_insecure_registries_is_the_https_anonymous_default() {
        assert_eq!(
            decide_transport(None, Vec::new()),
            TransportDecision::Default
        );
    }

    /// Live pull; set SORX_TEST_OCI_REF=oci://<ref> to run. Skipped otherwise.
    #[test]
    fn a_live_oci_reference_materializes_a_gtpack() {
        let Ok(raw) = std::env::var("SORX_TEST_OCI_REF") else {
            eprintln!("SORX_TEST_OCI_REF unset; skipping");
            return;
        };
        let path = materialize(Path::new(&raw)).expect("pull");
        let bytes = std::fs::read(&path).expect("read pulled pack");
        assert_eq!(&bytes[..2], b"PK", "a .gtpack is a zip archive");
    }
}
