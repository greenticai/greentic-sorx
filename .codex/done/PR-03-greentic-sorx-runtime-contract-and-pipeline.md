# PR 03 — greentic-sorx: gtc env runtime-host contract, traffic routing, and ext-pack deployment

## Repo

`greenticai/greentic-sorx`

## Goal

Make Sorx implement the generic runtime-host contract so it can be selected by a `gtc op env` deployer binding, driven by generic environment lifecycle operations, and shipped as an ext-pack/deployer pack without `gtc`, setup, start, operator, or deployer needing Sorx-specific branches.

The end state is:

- `greentic-sorx` can stage, warm, activate, route traffic for, drain, deactivate, and report health for multiple deployments/revisions in one runtime.
- `greentic-sorx` can consume the deployer/environment model (`Environment`, `BundleDeployment`, `Revision`, `TrafficSplit`, `RuntimeConfig`) as the external source of truth.
- `greentic-sorx` publishes a deployer/runtime-host ext-pack descriptor that can be bound into an environment slot and resolved by the generic deployer mechanism.
- Existing Sorx/Sorla-specific APIs may remain for compatibility, but generic admin/runtime endpoints are the contract used by `gtc op env`.

## Validated upstream assumptions

The first two PR specs have been folded into this PR as dependency assumptions. Current sibling repo checks on `develop` show:

- `greentic-deployer` already has `greentic-deploy-spec` with `Environment`, `EnvPackBinding`, `CapabilitySlot`, `PackDescriptor`, `BundleDeployment`, `Revision`, `TrafficSplit`, `RuntimeConfig`, `LocalFsStore`, and `EnvPackHandler`.
- `CapabilitySlot` is a closed enum with `deployer`, `secrets`, `telemetry`, `sessions`, `state`, and `revocation`. Sorx must bind through `CapabilitySlot::Deployer`; do not add a Sorx-specific slot.
- `PackDescriptor` is open-form `<namespace>.<id>@<semver>`. Sorx should use a descriptor path such as `greentic.deployer.sorx` or `greentic.deployer.runtime-host`, with the final value matching the deployer registry.
- `EnvPackHandler` is currently metadata/preflight only. The deployment lifecycle trait from PR2 is still an upstream prerequisite; this PR must either depend on it or provide the Sorx-side implementation once it lands.
- `RevisionLifecycle` currently supports `Inactive -> Staged -> Warming -> Ready -> Draining -> Inactive -> Archived`, plus failure/archive paths. Sorx lifecycle endpoints must map to these exact states.
- `RuntimeConfig` currently contains `env_id` and `RevisionRuntimeBlock { deployment_id, revision_id, bundle_id, pack_list_refs, pack_config_refs, weight_bps }`. Sorx must load this shape, not invent another environment store.
- `greentic` does not yet expose the generic capability/contract IDs from PR1 as code. Sorx can declare those IDs in its ext-pack manifest and admin response, but compile-time coupling should wait for the upstream crate/API.
- `greentic-start` already has a materialized `runtime-config.json` loader and revision dispatcher concepts. Sorx should interoperate with that environment materialization rather than duplicating it.
- `greentic-setup` persists deployment targets and validates pack capabilities through `greentic.ext.capabilities.v1`; Sorx's ext-pack must include that capabilities extension so setup/operator can discover it.

## Do not do

- Do not require `gtc`, `setup`, `start`, `bundle`, `operator`, or `deployer` to know Sorx internals.
- Do not expose Sorla-specific type names in public generic contracts.
- Do not invent a parallel environment store.
- Do not add a Sorx-specific `CapabilitySlot`.
- Do not key dispatch on the string `sorx` outside Sorx-owned descriptor/manifest metadata.
- Do not treat Sorx's existing deployment registry as the authoritative environment store; it is a runtime-local snapshot/cache.

## Upstream dependency contract

This PR assumes the deployer work provides, or will provide, a generic deployment lifecycle handler layered on top of `EnvPackHandler`:

```rust
pub trait DeploymentEnvPackHandler: EnvPackHandler {
    fn stage_revision(&self, ctx: &DeployContext, req: StageRevisionRequest)
        -> Result<StageRevisionResult, DeployError>;
    fn warm_revision(&self, ctx: &DeployContext, req: WarmRevisionRequest)
        -> Result<WarmRevisionResult, DeployError>;
    fn activate_revision(&self, ctx: &DeployContext, req: ActivateRevisionRequest)
        -> Result<ActivateRevisionResult, DeployError>;
    fn set_traffic_split(&self, ctx: &DeployContext, req: SetTrafficSplitRequest)
        -> Result<SetTrafficSplitResult, DeployError>;
    fn drain_revision(&self, ctx: &DeployContext, req: DrainRevisionRequest)
        -> Result<DrainRevisionResult, DeployError>;
    fn runtime_health(&self, ctx: &DeployContext)
        -> Result<RuntimeHealth, DeployError>;
}
```

If the final deployer API lands with different type names, adapt the Sorx implementation to that API while preserving the behavior above.

## Ext-pack descriptor and capabilities

Ship Sorx as an environment/deployer ext-pack with a descriptor path agreed with `greentic-deployer`:

```text
greentic.deployer.sorx@0.1.0
```

The ext-pack manifest must include `greentic.ext.capabilities.v1` and declare:

```yaml
schema: greentic.capabilities.v1
offers:
  - capability: greentic.cap.runtime.host.v1
    contracts:
      - greentic.runtime.admin.v1
      - greentic.runtime.health.v1
      - greentic.runtime.deployments.v1
      - greentic.runtime.traffic.v1
      - greentic.runtime.invoke.v1
requires:
  - capability: greentic.cap.secrets.v1
    optional: true
  - capability: greentic.cap.telemetry.v1
    optional: true
  - capability: greentic.cap.extension.control.v1
    optional: true
  - capability: greentic.cap.extension.observer.v1
    optional: true
  - capability: greentic.cap.extension.admin.v1
    optional: true
```

The descriptor must be usable from an environment binding:

```json
{
  "slot": "deployer",
  "kind": "greentic.deployer.sorx@0.1.0",
  "pack_id": "greentic-sorx-runtime-host"
}
```

Sorx may return `"implementation": "sorx"` from its own admin metadata, but external dispatch must key on `CapabilitySlot::Deployer`, descriptor path, and generic contracts.

## Environment/deploy-spec integration

Use deploy-spec and deployer state as the external contract:

- Load `Environment`/`RuntimeConfig` provided by the operator/deployer.
- Treat `BundleDeployment`, `Revision`, and `TrafficSplit` as the canonical deployment/revision/traffic records.
- Store only runtime-local active snapshots in Sorx: loaded artifacts, route tables, hook bindings, admin bindings, and readiness/drain status.
- Map lifecycle calls to deploy-spec states:
  - `stage` creates or accepts `RevisionLifecycle::Staged` and prepares artifact metadata.
  - `warm` moves runtime work through `Warming` and reports readiness for `Ready`.
  - `activate` makes only ready revisions eligible for traffic.
  - `traffic` accepts basis-point weights from deploy-spec; do not use Sorx's older percent/header-only model for the generic contract.
  - `drain` stops new traffic for the revision, waits for in-flight work, and reports `Draining -> Inactive`.
  - `deactivate` removes active routing and leaves archival/purge decisions to the environment/deployer.
- Validate all env-relative artifact/config refs under the environment directory. Do not accept path traversal or absolute paths from runtime config.

## Runtime admin API

Implement generic runtime admin endpoints:

```http
GET  /admin/v1/runtime
GET  /admin/v1/health
GET  /admin/v1/capabilities
GET  /admin/v1/deployments
GET  /admin/v1/deployments/{deployment_id}
POST /admin/v1/deployments/stage
POST /admin/v1/deployments/{deployment_id}/warm
POST /admin/v1/deployments/{deployment_id}/activate
POST /admin/v1/deployments/{deployment_id}/traffic
POST /admin/v1/deployments/{deployment_id}/revisions/{revision_id}/drain
POST /admin/v1/deployments/{deployment_id}/deactivate
```

Use generic names in schemas:

- runtime
- deployment
- revision
- stack
- traffic
- control
- observer
- admin

## Runtime metadata response

```json
{
  "schema": "greentic.runtime.info.v1",
  "runtime_id": "runtime-main",
  "runtime_kind": "runtime-host",
  "implementation": "sorx",
  "version": "0.1.0",
  "contracts": [
    "greentic.runtime.admin.v1",
    "greentic.runtime.health.v1",
    "greentic.runtime.deployments.v1",
    "greentic.runtime.traffic.v1",
    "greentic.runtime.invoke.v1"
  ]
}
```

`implementation: sorx` is allowed inside Sorx responses, but callers should key on contracts.

## Runtime state

Sorx keeps a runtime-local view of:

- deployments
- revisions
- artifact locations
- routes
- traffic splits in basis points
- control hook bindings
- observer subscription bindings
- admin surface bindings
- secret bindings
- telemetry config

The source of truth remains environment/deployer state; Sorx holds the active runtime snapshot.

Existing Sorx deployment registry types may be adapted internally, but the generic API should expose `deployment_id`, `revision_id`, `bundle_id`, `stack_id`, lifecycle, and `weight_bps` rather than Sorx-only labels as the primary fields.

## Invocation pipeline

Every stack call runs:

```text
observer.pre_call
control.pre_call
stack.invoke
control.post_call
observer.post_call
```

If `control.pre_call` denies, do not invoke stack.

If observer fails and fail mode is open, record warning and continue.

If control fails closed, return controlled error.

## Generic call types

Use generic names:

- `StackCallContext`
- `StackCallRequest`
- `StackCallResponse`
- `ControlDecision`
- `ObserverEvent`
- `AdminSurface`

Avoid public names like `SorlaCallContext`.

## Control decisions

MVP:

```text
allow
deny
allow_with_patch
```

Future:

```text
route_override
require_approval
replace_response
quarantine
```

## Observer events

MVP event types:

```text
stack.call.started
stack.call.completed
stack.call.failed
stack.call.denied
admin.action.started
admin.action.completed
admin.action.denied
```

Common fields:

- environment_id
- runtime_id
- tenant_id
- team_id
- deployment_id
- bundle_id / stack_id
- revision_id
- route_id
- call_id
- trace_id
- actor metadata
- duration/status for post events
- control decisions for post events

## Admin surfaces

Support admin packs registering:

- admin APIs
- admin pages/assets
- admin actions
- navigation entries
- required control actions/permissions

Admin request pipeline:

```text
admin auth
observer.admin_action_started
control.pre_admin
admin handler/page/API
control.post_admin
observer.admin_action_completed
response
```

Admin APIs must not bypass control.

## Telemetry

Use `greentic-telemetry`.

- Use config-first initialisation where runtime config provides export config.
- Use env-driven auto config only as local/dev fallback.
- Add span attrs:
  - environment_id
  - runtime_id
  - tenant_id
  - team_id
  - deployment_id
  - revision_id
  - route_id
  - call_id
  - traffic selection
  - control decisions
  - observer counts
  - admin action id if relevant

Do not duplicate secret redaction.

## Backward compatibility

Keep existing Sorx routes such as `/v1/sorx/routes`, `/v1/sorx/tools`, `/v1/sorx/business-actions`, and existing pack loading behavior working unless a later PR explicitly removes them. The new generic admin API should be additive and can share internals with those routes.

Existing type names that include Sorx/Sorla may remain in private/internal modules. New public generic runtime contract types should use names such as:

- `RuntimeDeployment`
- `RuntimeRevision`
- `RuntimeTrafficSplit`
- `StackCallContext`
- `StackCallRequest`
- `StackCallResponse`
- `ControlDecision`
- `ObserverEvent`
- `AdminSurface`

## Implementation order

1. Add runtime contract models and serde tests for metadata, capabilities, deployments, revisions, traffic, lifecycle requests, stack calls, control decisions, observer events, and admin surfaces.
2. Add a runtime snapshot store that can be built from deploy-spec `Environment`/`RuntimeConfig` and Sorx's artifact loading path.
3. Add generic admin endpoints behind the existing HTTP runtime/admin server path.
4. Implement stage/warm/activate/traffic/drain/deactivate against the runtime snapshot and existing Sorx pack loading.
5. Add basis-point traffic routing over ready revisions only.
6. Add control/observer hooks around stack invocation.
7. Add admin-surface registration and route all admin actions through control/observer hooks.
8. Package the Sorx runtime-host/deployer ext-pack with capabilities metadata.
9. Wire the Sorx-side `DeploymentEnvPackHandler` implementation once the deployer lifecycle trait is available.
10. Add end-to-end coverage with a local environment binding using `greentic.deployer.sorx@0.1.0`.

## Tests

- Ext-pack manifest includes `greentic.ext.capabilities.v1`, offers `greentic.cap.runtime.host.v1`, and declares the runtime contracts.
- Environment binding through `CapabilitySlot::Deployer` resolves Sorx by descriptor path without adding a Sorx-specific slot.
- Runtime config loader accepts deploy-spec `RuntimeConfig` with multiple deployments/revisions.
- Runtime config rejects absolute paths, `..`, duplicate revision IDs per deployment, mixed bundles per deployment, and traffic weights that do not sum to 10,000 bps per deployment.
- Stage/warm/activate deployment through admin API.
- Traffic route chooses only ready revisions with positive weight.
- Revision with zero/absent weight receives no traffic.
- Draining revision receives no new traffic.
- Deactivate removes active routes without purging env/deployer state.
- Control pre deny prevents stack invocation.
- Control pre patch mutates request.
- Control post patch mutates response.
- Observer pre/post receives events.
- Observer failure fail-open continues.
- Admin pack registers an API and page dynamically.
- Admin API runs through control.
- Telemetry spans include environment/runtime/deployment/revision/call fields.
- Existing `/v1/sorx/*` routes still work.

## Acceptance criteria

Sorx can be driven through generic runtime contracts by `gtc op env`, host multiple stack deployments/revisions in parallel, move traffic by deploy-spec basis-point splits, run control/observer hooks, mount admin surfaces, and be discovered/deployed as a capabilities-bearing ext-pack without product-specific branching in setup/start/operator/deployer.
