//! Where a `start` pack comes from: a local file, or an OCI reference pulled
//! at boot (so a container needs no volume for its pack).

use std::path::{Path, PathBuf};

use greentic_distributor_client::{
    OciPackFetcher, PackFetchOptions, oci_packs::DefaultRegistryClient,
};

use crate::ar_token;
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
/// in this module's tests). Credentials, in order: `OCI_USERNAME` /
/// `OCI_PASSWORD` (the pair greentic-start honours, so one Kubernetes Secret
/// serves both); otherwise, for an Artifact Registry host (`*-docker.pkg.dev`),
/// the attached service account's token from the GCP metadata server as
/// `oauth2accesstoken` (see `ar_token`); otherwise an anonymous pull.
///
/// A tag-only reference resolved onto a plain-HTTP registry (via
/// `GREENTIC_OCI_INSECURE_REGISTRIES`) is refused before any network call —
/// see `digest_required_for_plain_http`.
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
        cache_dir: pack_cache_dir(),
        ..PackFetchOptions::default()
    };
    let insecure_registries = insecure_registries_from_env();
    let host = reference_host(&reference);
    let credentials =
        resolve_pull_credentials(&host, pull_credentials(), ar_token::artifact_registry_token);
    let source = credentials.as_ref().map(|credentials| credentials.source);
    if source == Some(CredentialSource::ArtifactRegistryMetadata) {
        eprintln!(
            "greentic-sorx: pulling oci://{reference} with {}",
            CredentialSource::ArtifactRegistryMetadata.describe()
        );
    }
    let decision = decide_transport(credentials, insecure_registries);
    if let TransportDecision::InsecureRegistries(ref registries) = decision
        && let Some(err) = digest_required_for_plain_http(&reference, registries)
    {
        return Err(err);
    }
    let fetcher: OciPackFetcher<DefaultRegistryClient> = match decision {
        TransportDecision::Default => OciPackFetcher::new(options),
        TransportDecision::Authenticated {
            credentials,
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
                DefaultRegistryClient::with_basic_auth(credentials.username, credentials.password),
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
        .map_err(|err| {
            CliError::runtime(pull_error_message(&reference, &err.to_string(), source))
        })?;
    Ok(fetched.path)
}

/// Where a pulled pack's cache lives.
///
/// `greentic_distributor_client::oci_packs::default_cache_root` already
/// honours `GREENTIC_PACK_CACHE_DIR` when it is set, but falls back to
/// `dirs_next::cache_dir()` (`$HOME/.cache/...`) when it is not — and under
/// contract C3 (read-only rootfs, only `/tmp` writable) that resolves to a
/// path the process cannot create, so every pull fails with
/// `io error while caching … Read-only file system (os error 30)` even
/// though nobody asked for a `$HOME`-rooted cache. `std::env::temp_dir()` is
/// `/tmp` on every target this binary ships for, which the container always
/// provides as a writable `emptyDir`, so that is the fallback here instead of
/// the library's `$HOME`-based one. `GREENTIC_PACK_CACHE_DIR` still wins when
/// set, matching the library's own precedence.
fn pack_cache_dir() -> PathBuf {
    pack_cache_dir_from(std::env::var("GREENTIC_PACK_CACHE_DIR").ok())
}

/// Pure decision behind [`pack_cache_dir`], taking the env var's value
/// (already read) so it can be unit-tested without mutating process state.
fn pack_cache_dir_from(override_dir: Option<String>) -> PathBuf {
    match override_dir {
        Some(root) if !root.is_empty() => PathBuf::from(root),
        _ => std::env::temp_dir().join("greentic-sorx-packs"),
    }
}

/// Which registry client to build for a pull, decided once so the "explicit
/// credentials always win" rule is a pure function callers and tests can
/// reason about without a network call.
///
/// Mirrors greentic-start's `fetch_remote_bundle`: `DefaultRegistryClient`'s
/// `with_basic_auth` and `with_insecure_registries` constructors each
/// hardcode the OTHER axis (auth vs. transport), so a pull carrying any
/// credential must not be silently downgraded to plain HTTP by an also-set
/// `GREENTIC_OCI_INSECURE_REGISTRIES` — that would send credentials in the
/// clear to whichever registry the pull resolves to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TransportDecision {
    /// No credentials, no insecure registries: HTTPS, anonymous.
    Default,
    /// Credentials (explicit, or the Artifact Registry metadata token) win
    /// outright; `insecure_registries_ignored` is carried through only so the
    /// caller can warn that it had no effect. `PullCredentials`' own `Debug`
    /// redacts the password, so this derive cannot leak it.
    Authenticated {
        credentials: PullCredentials,
        insecure_registries_ignored: Vec<String>,
    },
    /// No credentials: the listed `host[:port]` registries are pulled over
    /// plain HTTP, everything else stays HTTPS.
    InsecureRegistries(Vec<String>),
}

fn decide_transport(
    credentials: Option<PullCredentials>,
    insecure_registries: Vec<String>,
) -> TransportDecision {
    match credentials {
        Some(credentials) => TransportDecision::Authenticated {
            credentials,
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

/// Where a pull's credential came from — reported in a failed pull's message,
/// never the credential itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialSource {
    /// `OCI_USERNAME` / `OCI_PASSWORD`, set by the operator.
    Explicit,
    /// The attached service account's token from the GCP metadata server.
    ArtifactRegistryMetadata,
}

impl CredentialSource {
    fn describe(self) -> &'static str {
        match self {
            Self::Explicit => "OCI_USERNAME/OCI_PASSWORD",
            Self::ArtifactRegistryMetadata => {
                "the runtime service account's token from the GCP metadata server"
            }
        }
    }
}

/// A registry credential for one pull. `Debug` is written by hand so the
/// password can never reach a log line through `{:?}`.
#[derive(Clone, PartialEq, Eq)]
struct PullCredentials {
    username: String,
    password: String,
    source: CredentialSource,
}

impl std::fmt::Debug for PullCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PullCredentials")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("source", &self.source)
            .finish()
    }
}

/// Which credential authenticates a pull from `host`.
///
/// Explicit wins: an operator who set `OCI_USERNAME`/`OCI_PASSWORD` must not
/// be silently overridden by a GCP token that merely happens to be ambient.
/// `ar_token` is only invoked when `explicit` is `None` — a pure function of
/// its inputs, so the precedence is tested without a metadata server
/// (mirrors greentic-start `resolve_pull_credentials`).
fn resolve_pull_credentials(
    host: &str,
    explicit: Option<(String, String)>,
    ar_token: impl FnOnce(&str) -> Option<String>,
) -> Option<PullCredentials> {
    match explicit {
        Some((username, password)) => Some(PullCredentials {
            username,
            password,
            source: CredentialSource::Explicit,
        }),
        None => ar_token(host).map(|token| PullCredentials {
            // Full path: the closure parameter `ar_token` shadows the module here.
            username: crate::ar_token::AR_USERNAME.to_string(),
            password: token,
            source: CredentialSource::ArtifactRegistryMetadata,
        }),
    }
}

/// The message for a failed pull. Names which credential was used — never its
/// value — and, for the metadata token, the grant whose absence is the usual
/// cause, so the deployer can report something actionable.
fn pull_error_message(reference: &str, error: &str, source: Option<CredentialSource>) -> String {
    match source {
        None => format!("cannot pull pack oci://{reference}: {error} (pulled anonymously)"),
        Some(CredentialSource::Explicit) => format!(
            "cannot pull pack oci://{reference}: {error} (pulled with {})",
            CredentialSource::Explicit.describe()
        ),
        Some(CredentialSource::ArtifactRegistryMetadata) => format!(
            "cannot pull pack oci://{reference}: {error} (pulled with {}; if the registry \
             refused it, grant that service account roles/artifactregistry.reader on the \
             repository)",
            CredentialSource::ArtifactRegistryMetadata.describe()
        ),
    }
}

/// Read the `GREENTIC_OCI_INSECURE_REGISTRIES` allow-list (comma-separated
/// `host[:port]` entries pulled over plain HTTP instead of HTTPS). Unset or
/// empty yields an empty list (HTTPS for every registry, the default).
///
/// Unlike greentic-start's `insecure_registries_for_fetch`, `materialize` has
/// exactly one pull site, so there is no separate "non-boot resolution"
/// caller to keep HTTPS-only — the env var is honoured unconditionally here.
/// That does **not** mean every insecure pull is digest-gated automatically:
/// digest verification only happens when the REFERENCE itself pins one, and a
/// tag-only reference resolved onto one of these registries carries no such
/// pin. `digest_required_for_plain_http` is what closes that gap — a
/// plain-HTTP pull with no digest is refused before the network call, rather
/// than trusted.
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

/// The registry host a bare OCI reference (no `oci://` scheme) actually
/// pulls from. Used to test a reference's host against the plain-HTTP
/// allow-list.
///
/// Parses with `oci_client::Reference` (re-exported through
/// `greentic_distributor_client`, which this crate already depends on — no
/// new dependency needed) and reads `resolve_registry()`, the same call the
/// fetcher's transport decision (`ClientProtocol::scheme_for`) makes. A naive
/// `split('/')` on the leading segment agrees with it for an ordinary
/// `host[:port]` registry, but diverges for `docker.io`, which
/// `resolve_registry()` redirects to `index.docker.io` — the actual host the
/// pull's scheme is chosen for. A reference this parser rejects (should not
/// happen; `parse_pack_ref` only strips a scheme, it does not validate) falls
/// back to the naive split rather than panicking.
fn reference_host(reference: &str) -> String {
    match reference.parse::<greentic_distributor_client::oci_client::Reference>() {
        Ok(parsed) => parsed.resolve_registry().to_string(),
        Err(_) => reference.split('/').next().unwrap_or(reference).to_string(),
    }
}

/// A plain-HTTP registry pull has no transport integrity — anything on the
/// path can serve different bytes — so it is safe only when the reference
/// itself pins a digest the fetcher then verifies
/// (`OciPackError::DigestMismatch` on a mismatch, see this module's doc
/// comment). A tag-only reference resolved onto an insecure registry is
/// refused up front instead: the same trade-off greentic-start makes for its
/// own plain-HTTP pulls.
///
/// Returns `None` when the reference already carries a `@sha256:` digest, or
/// when its host is not on the plain-HTTP allow-list (HTTPS still applies,
/// where a tag-only reference is fine — the fetcher trusts the certificate
/// chain instead).
fn digest_required_for_plain_http(
    reference: &str,
    insecure_registries: &[String],
) -> Option<CliError> {
    if reference.contains("@sha256:") {
        return None;
    }
    let host = reference_host(reference);
    if !insecure_registries.contains(&host) {
        return None;
    }
    Some(CliError::runtime(format!(
        "cannot pull pack oci://{reference}: {host} is pulled over plain HTTP via \
         GREENTIC_OCI_INSECURE_REGISTRIES, which requires a digest pin (…@sha256:<hex>) so an \
         unauthenticated plain-HTTP registry cannot silently serve different bytes"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ar_token::mock_metadata::{refused, serve_once};
    use std::path::Path;
    use std::time::Duration;

    fn explicit(user: &str, pass: &str) -> PullCredentials {
        PullCredentials {
            username: user.to_string(),
            password: pass.to_string(),
            source: CredentialSource::Explicit,
        }
    }

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
    fn pack_cache_dir_falls_back_to_the_temp_dir_when_unset() {
        assert_eq!(
            pack_cache_dir_from(None),
            std::env::temp_dir().join("greentic-sorx-packs")
        );
    }

    #[test]
    fn pack_cache_dir_honours_the_env_override_when_set() {
        assert_eq!(
            pack_cache_dir_from(Some("/custom/pack/cache".to_string())),
            PathBuf::from("/custom/pack/cache")
        );
    }

    #[test]
    fn pack_cache_dir_falls_back_when_the_override_is_empty() {
        assert_eq!(
            pack_cache_dir_from(Some(String::new())),
            std::env::temp_dir().join("greentic-sorx-packs")
        );
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
        let decision =
            decide_transport(Some(explicit("u", "p")), vec!["localhost:5000".to_string()]);
        assert_eq!(
            decision,
            TransportDecision::Authenticated {
                credentials: explicit("u", "p"),
                insecure_registries_ignored: vec!["localhost:5000".to_string()],
            }
        );
    }

    #[test]
    fn explicit_credentials_win_and_the_metadata_server_is_never_asked() {
        let resolved = resolve_pull_credentials(
            "europe-west1-docker.pkg.dev",
            Some(("u".to_string(), "p".to_string())),
            |_| panic!("explicit credentials must short-circuit the metadata token"),
        );
        assert_eq!(resolved, Some(explicit("u", "p")));
    }

    #[test]
    fn an_ar_host_without_explicit_credentials_pulls_as_oauth2accesstoken() {
        let server = serve_once(200, r#"{"access_token":"ya29.from-metadata"}"#);
        let url = server.url.clone();
        let resolved = resolve_pull_credentials("europe-west1-docker.pkg.dev", None, |host| {
            crate::ar_token::artifact_registry_token_from(host, &url, Duration::from_secs(5))
        });
        assert_eq!(
            resolved,
            Some(PullCredentials {
                username: "oauth2accesstoken".to_string(),
                password: "ya29.from-metadata".to_string(),
                source: CredentialSource::ArtifactRegistryMetadata,
            })
        );
    }

    #[test]
    fn an_unreachable_metadata_server_falls_back_to_the_anonymous_default() {
        let url = refused();
        let resolved = resolve_pull_credentials("europe-west1-docker.pkg.dev", None, |host| {
            crate::ar_token::artifact_registry_token_from(host, &url, Duration::from_secs(5))
        });
        assert_eq!(resolved, None);
        assert_eq!(
            decide_transport(resolved, Vec::new()),
            TransportDecision::Default
        );
    }

    #[test]
    fn a_non_ar_reference_resolves_to_no_credentials_and_never_asks_for_a_token() {
        let server = serve_once(200, r#"{"access_token":"ya29.must-not-leak"}"#);
        let url = server.url.clone();
        let host = reference_host("ghcr.io/greenticai/sor-landlord:t1");
        let resolved = resolve_pull_credentials(&host, None, |host| {
            crate::ar_token::artifact_registry_token_from(host, &url, Duration::from_secs(5))
        });
        assert_eq!(resolved, None);
        assert!(
            server
                .seen
                .recv_timeout(Duration::from_millis(300))
                .is_err()
        );
    }

    #[test]
    fn the_host_comes_from_the_registry_not_from_a_path_segment() {
        assert_eq!(
            reference_host("europe-west1-docker.pkg.dev/proj/repo/sorla/landlord:t1@sha256:ab"),
            "europe-west1-docker.pkg.dev"
        );
        assert_eq!(
            reference_host("reg.example/attacker-docker.pkg.dev/sor:t1"),
            "reg.example"
        );
    }

    #[test]
    fn pull_credentials_debug_never_prints_the_password() {
        let creds = PullCredentials {
            username: "oauth2accesstoken".to_string(),
            password: "ya29.super-secret".to_string(),
            source: CredentialSource::ArtifactRegistryMetadata,
        };
        let rendered = format!("{creds:?}");
        assert!(!rendered.contains("ya29.super-secret"), "{rendered}");
        assert!(rendered.contains("oauth2accesstoken"), "{rendered}");
        let decision = decide_transport(Some(creds), Vec::new());
        assert!(!format!("{decision:?}").contains("ya29.super-secret"));
    }

    #[test]
    fn a_failed_ar_token_pull_names_the_source_and_the_reader_grant() {
        let message = pull_error_message(
            "europe-west1-docker.pkg.dev/p/r/sorla/landlord:t1",
            "401 Unauthorized",
            Some(CredentialSource::ArtifactRegistryMetadata),
        );
        assert!(
            message.contains("oci://europe-west1-docker.pkg.dev/p/r/sorla/landlord:t1"),
            "{message}"
        );
        assert!(message.contains("401 Unauthorized"), "{message}");
        assert!(message.contains("metadata server"), "{message}");
        assert!(
            message.contains("roles/artifactregistry.reader"),
            "{message}"
        );
    }

    #[test]
    fn a_failed_anonymous_or_explicit_pull_keeps_its_plain_message() {
        let anonymous = pull_error_message("ghcr.io/x/y:t", "boom", None);
        assert!(anonymous.contains("pulled anonymously"), "{anonymous}");
        assert!(!anonymous.contains("artifactregistry"), "{anonymous}");
        let explicit_msg =
            pull_error_message("ghcr.io/x/y:t", "boom", Some(CredentialSource::Explicit));
        assert!(
            explicit_msg.contains("OCI_USERNAME/OCI_PASSWORD"),
            "{explicit_msg}"
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

    #[test]
    fn a_tag_only_reference_is_refused_on_a_plain_http_registry() {
        let err = digest_required_for_plain_http(
            "localhost:5000/greentic/sor-landlord:t1",
            &["localhost:5000".to_string()],
        )
        .expect("a tag-only reference on the insecure allow-list must be refused");
        assert!(
            err.message
                .contains("oci://localhost:5000/greentic/sor-landlord:t1"),
            "{}",
            err.message
        );
        assert!(err.message.contains("digest pin"), "{}", err.message);
    }

    #[test]
    fn a_digest_pinned_reference_is_allowed_on_a_plain_http_registry() {
        assert_eq!(
            digest_required_for_plain_http(
                "localhost:5000/greentic/sor-landlord:t1@sha256:ab",
                &["localhost:5000".to_string()],
            ),
            None
        );
    }

    #[test]
    fn reference_host_follows_the_fetchers_own_registry_resolution() {
        // An ordinary private registry: naive split and Reference agree.
        assert_eq!(
            reference_host("localhost:5000/greentic/sor-landlord:t1"),
            "localhost:5000"
        );
        // docker.io is redirected to index.docker.io by
        // `Reference::resolve_registry()` — the same host the fetcher's
        // transport decision is made against — so the gate must follow it
        // rather than the literal leading path segment.
        assert_eq!(
            reference_host("docker.io/greentic/sor-landlord:t1"),
            "index.docker.io"
        );
    }

    #[test]
    fn a_tag_only_reference_off_the_insecure_allow_list_is_not_refused() {
        // HTTPS still applies to this host, where the fetcher trusts the
        // certificate chain instead of a pinned digest.
        assert_eq!(
            digest_required_for_plain_http(
                "reg.example/greentic/sor-landlord:t1",
                &["localhost:5000".to_string()],
            ),
            None
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
