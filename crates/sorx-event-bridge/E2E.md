# sorx-event-bridge — End-to-End Runbook

Manual runbook for verifying the full NATS round-trip across three services:
**NATS broker → greentic-sorx (event bridge) → greentic-runner (sorla.call node)**.

---

## Prerequisites

| Tool | Minimum version |
|------|----------------|
| Rust / Cargo | 1.90+ (see `rust-toolchain.toml`) |
| Docker | Any recent version (for the NATS broker) |
| `nats` CLI (optional) | For traffic inspection (`nats sub`) |
| A compiled `greentic-sorx` binary | `cargo build -p greentic-sorx-cli --release` |
| A compiled `greentic-runner` binary | From the `greentic-runner` repository |
| A SoRLa `.gtpack` file | Any pack that exposes at least one HTTP endpoint |
| A `.ygtc` flow file | Containing a `sorla.call` node (see Step 3) |

---

## Step 1 — Start NATS

The bridge and runner must both connect to the same NATS broker.

**Docker (recommended for local testing):**

```bash
docker run --rm -p 4222:4222 nats:latest
```

**Or use a local `nats-server` binary:**

```bash
nats-server
```

NATS listens on `nats://127.0.0.1:4222` by default.

**Verify the broker is reachable** (requires the `nats` CLI):

```bash
nats pub greentic.test "ping"
```

---

## Step 2 — Start the sorx runtime with the event bridge

The bridge auto-starts when `GREENTIC_EVENTS_NATS_URL` is set in the
environment. No extra flag is needed.

```bash
GREENTIC_EVENTS_NATS_URL=nats://127.0.0.1:4222 \
  greentic-sorx start <your-pack.gtpack> --answers <answers.json>
```

For the landlord/tenant fixture bundled in this repo:

```bash
GREENTIC_EVENTS_NATS_URL=nats://127.0.0.1:4222 \
  greentic-sorx start crates/greentic-sorx-cli/tests/e2e/fixtures/landlord_tenant/landlord.gtpack \
  --answers crates/greentic-sorx-cli/tests/e2e/fixtures/landlord_tenant/landlord.answers.json
```

You should see a log line like:

```
greentic-sorx: event bridge connected to NATS at nats://127.0.0.1:4222 (greentic.sorla.request.v1)
```

If the env var is absent or empty the bridge is silently skipped — HTTP
serving continues normally.

**Confirm the endpoint is available** (verify against `greentic-sorx routes
<pack> --json` for actual route paths):

```bash
curl -s http://127.0.0.1:<PORT>/v1/sorx/<tenant>/<sor>/tenants \
  -H "Content-Type: application/json" \
  -d '{"id":"t-1","name":"Alice","contact":"alice@example.com"}'
```

---

## Step 3 — Start greentic-runner with a flow containing a `sorla.call` node

The runner dispatches to sorx by publishing on `greentic.sorla.request.v1`
and waiting for the response on `greentic.sorla.response.v1`.

Set the same NATS URL in the runner's environment:

```bash
GREENTIC_EVENTS_NATS_URL=nats://127.0.0.1:4222 \
  greentic-runner start --bundle <bundle-dir>
```

Verify against the `greentic-runner` CLI help for the exact start command and
bundle layout, as the runner's CLI may differ from the sorx CLI.

### Example flow fragment (`.ygtc`)

A `sorla.call` node dispatches an operation to the sorx runtime. The node
schema is:

```yaml
nodes:
  - id: call_sorx
    component: sorla.call
    config:
      operation: "tenants.create"   # must match a sorx endpoint operation name
      target: "landlord"            # the SoR identifier from the pack
      await: true                   # wait for the response before continuing
    input:
      id: "{{ trigger.body.tenant_id }}"
      name: "{{ trigger.body.name }}"
```

The runtime serialises this to a `RuntimeDispatchRequest`:

```json
{
  "target": "landlord",
  "operation": "tenants.create",
  "mode": "await",
  "input": { "id": "...", "name": "..." },
  "deadline_ms": 30000
}
```

It publishes this to `greentic.sorla.request.v1` with headers:

| Header | Value |
|--------|-------|
| `Greentic-Correlation-Id` | `<bare hint>::pack=<pack>::flow=<flow>` |
| `Greentic-Tenant` | tenant id from the flow context |
| `Greentic-Env` | environment id (e.g. `default`) |

---

## Step 4 — Trigger the flow and observe the round-trip

Trigger the flow via its inbound channel (HTTP endpoint, messaging provider,
webhook, etc.) as configured in the bundle.

**What you expect to observe:**

1. A message appears on `greentic.sorla.request.v1` — the runner dispatched
   the `sorla.call` node.
2. The sorx runtime receives the message, invokes the endpoint, and publishes a
   `RuntimeDispatchResponse` to `greentic.sorla.response.v1`.
3. The runner receives the response, resumes the suspended flow step, and
   continues execution.

**Watch traffic in real time** using the `nats` CLI:

```bash
nats sub 'greentic.sorla.>'
```

This subscribes to both `greentic.sorla.request.v1` and
`greentic.sorla.response.v1` with a single wildcard subscription.

---

## Step 5 — Verify the response

The response JSON on `greentic.sorla.response.v1` has this shape:

```json
{
  "ok": true,
  "output": { ... },
  "events": []
}
```

An error response looks like:

```json
{
  "ok": false,
  "output": null,
  "events": [],
  "error": {
    "code": "invoke_failed",
    "message": "..."
  }
}
```

The `Greentic-Correlation-Id` header on the response message must match the id
from the request exactly. The runner uses this to route the response back to the
correct suspended flow instance.

---

## Correlation id format

The runner generates correlation ids with this pattern:

```
<bare-hint>::pack=<pack-id>::flow=<flow-id>
```

Example:

```
sorla.call.0::pack=landlord-v0.1.9::flow=create-tenant-flow
```

This is a hint only — the bridge echoes it verbatim without parsing. The runner
is the sole owner of the mapping from correlation id to suspended flow instance.

---

## Known limitation — inbound-reply resumption

When a flow was itself triggered by an inbound NATS message that carries a
non-empty `reply_to` or `thread_id`, and that flow then makes a `sorla.call`
with `await: true`, the correlation id alone is not sufficient to resume the
original inbound context. The runner must hold the inbound message open (or
re-publish to the original reply address) after the sorla response arrives.
This re-entry path is not yet implemented. Workaround: use `await: false`
(fire-and-forget) for `sorla.call` nodes inside flows that originate from
message-reply contexts, and handle the result via a separate trigger.

---

## Running the gated integration test against a live broker

```bash
# Terminal 1: start NATS
docker run --rm -p 4222:4222 nats:latest

# Terminal 2: run the test
GREENTIC_TEST_NATS_URL=nats://127.0.0.1:4222 \
  cargo test -p sorx-event-bridge --test nats_roundtrip -- --nocapture
```

Expected output:

```
running 1 test
test nats_roundtrip_sorla_request_response ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;
```

Without the env var set, the test self-skips in under 1 ms:

```
running 1 test
SKIP nats_roundtrip: GREENTIC_TEST_NATS_URL not set — skipping (no broker)
test nats_roundtrip_sorla_request_response ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out;
```

---

## Debugging tips

| Problem | Check |
|---------|-------|
| No message on `greentic.sorla.request.v1` | Confirm runner has `GREENTIC_EVENTS_NATS_URL` set and connected to NATS |
| Bridge not consuming requests | Confirm sorx has `GREENTIC_EVENTS_NATS_URL` set; check for the "event bridge connected" log line |
| Response arrives but flow does not resume | Correlation id mismatch; check `Greentic-Correlation-Id` header on both request and response messages |
| `invoke_failed` in response body | The sorx endpoint returned an error; check the sorx runtime logs for the operation trace |
| `timeout` in runner after 5+ seconds | NATS network partition or bridge crashed; check `nats sub 'greentic.sorla.>'` for traffic |
