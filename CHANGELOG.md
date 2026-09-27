# Changelog

## Unreleased

- `start`/`run` pull an `oci://<region>-docker.pkg.dev/...` pack with the
  attached service account's GCP metadata-server token (`oauth2accesstoken`)
  when `OCI_USERNAME`/`OCI_PASSWORD` are not set, so a Cloud Run sorx service
  needs no registry key. Explicit credentials still win; off GCP the pull stays
  anonymous. A failed pull now names which credential source it used.
- Added SORX runtime scaffold through PR 11, including pack loading, startup
  answers, HTTP runtime, MCP adapter metadata, provider bindings, policy,
  approvals, audit, local e2e coverage, `gtc` integration docs, and release
  hardening.
- Added business event publication: `BusinessEventSink` trait with
  `disabled` (default), `stdout`, and NATS sinks; canonical `EventEnvelope`
  emission on record create/update/delete and `emit_event` command steps;
  `events` config section in startup answers; topic sanitization following the
  `sorla.<pack>.<Entity>.<operation>` and `sorla.<pack>.<event_name>` schemes;
  NATS subjects as `<subject_prefix>.<tenant>.<topic>`; `NatsEventSink` behind
  the `events-nats` cargo feature (bounded queue 1024, best-effort
  at-most-once); business-event capability offers with `topic` metadata.
