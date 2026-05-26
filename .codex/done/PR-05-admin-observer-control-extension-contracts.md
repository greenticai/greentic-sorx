# PR 05 — greentic-sorx: extension pack bindings for control, observer, and admin surfaces

## Repo

`greenticai/greentic-sorx`

## Status from current code

The base runtime contract from PR 03 is already present in this repo as uncommitted work:

- `crates/greentic-sorx-core/src/generic_runtime.rs` defines generic runtime metadata, capabilities, deployment/revision/traffic models, stack/admin control hooks, observer events, admin surfaces, and runtime-config validation.
- `crates/greentic-sorx-core/src/runtime.rs` already runs stack calls through `pre_call`, `post_call`, and observer hooks.
- `crates/greentic-sorx-cli/src/http_runtime.rs` already exposes `/admin/v1/runtime`, `/admin/v1/health`, `/admin/v1/capabilities`, deployment lifecycle, runtime-config apply, and admin-surface registration/listing. Generic admin requests already run through `pre_admin`, `post_admin`, and observer events.
- `crates/greentic-sorx-pack/src/manifest.rs` already emits the Sorx runtime-host capability block for `greentic.deployer.sorx@0.1.0`.

This PR should not reimplement those pieces. It should finish the missing Sorx-owned part: bind extension packs from runtime config or pack metadata into the existing generic hook interfaces and admin surface registry.

## Goal

Make Sorx load and execute generic control, observer, and admin extension packs declared by the environment/runtime snapshot, without hardcoding extension implementations.

Sorx remains the runtime host. `greentic`, `greentic-start`, `greentic-setup`, and `greentic-bundle` own their own CLI, setup, start, and bundle validation behavior outside this repo.

## Runtime config extension bindings

Extend the Sorx-owned runtime config model to accept optional extension bindings while remaining compatible with the current `greentic.runtime-config.v1` shape:

```json
{
  "schema": "greentic.runtime-config.v1",
  "env_id": "local",
  "revisions": [],
  "extensions": {
    "control": {
      "hooks": {
        "pre_call": [
          {
            "id": "policy",
            "contract": "greentic.control.pre-call.v1",
            "pack_ref": "ghcr.io/greenticai/extensions/control/policy:1.0.0",
            "fail_mode": "closed"
          }
        ]
      }
    },
    "observer": {
      "subscriptions": {
        "post_call": [
          {
            "id": "audit",
            "contract": "greentic.observer.post-call.v1",
            "pack_ref": "ghcr.io/greenticai/extensions/observer/audit:1.0.0",
            "fail_mode": "open"
          }
        ]
      }
    },
    "admin": {
      "surfaces": [
        {
          "id": "stack-console",
          "contract": "greentic.admin.surface.v1",
          "pack_ref": "ghcr.io/greenticai/extensions/admin/stack-console:1.0.0",
          "mount": "/admin/stacks"
        }
      ]
    }
  }
}
```

Validation is generic:

- Known control hook names: `pre_call`, `post_call`, `pre_admin`, `post_admin`.
- Known observer subscriptions: `pre_call`, `post_call`, `call_failed`, `control_denied`, `admin_event`.
- Supported MVP control decisions remain `allow`, `deny`, and `allow_with_patch`.
- `fail_mode` is `open` or `closed`.
- Extension refs must be env-relative paths or accepted pack refs; local refs must not allow path traversal or absolute paths.
- Admin mount paths must be absolute, normalized, and collision-free within the runtime.

## Runtime execution

Implement an extension binding layer that adapts loaded extension packs to the existing traits:

- `ControlHook` for stack and admin control.
- `ObserverHook` for stack and admin observer events.
- `RuntimeSnapshot::register_admin_surface` for admin surface discovery.

The first implementation may use an in-process fake/test extension adapter plus pack metadata parsing. Do not require a specific third-party extension runtime until the extension pack execution API is available.

## Admin surfaces

The current API can register and list admin surfaces but does not mount pack-provided admin APIs/pages. Add Sorx-side support for:

- Loading an admin surface manifest from an extension pack.
- Registering pages/actions/APIs under the declared `mount`.
- Routing admin API calls through the existing generic admin pipeline.
- Returning a clear error for unsupported handler contracts rather than silently accepting them.

Static page serving can be minimal, but the manifest and route registration must be deterministic and covered by tests.

## Observer events still needed

The current runtime emits started/completed/denied events. Add missing failure coverage:

- Emit `stack.call.failed` when stack invocation fails after `observer.pre_call`.
- Emit `admin.action.failed` when a generic admin handler fails after `admin.action.started`.
- Preserve observer fail-open/fail-closed behavior per binding.

## Tests

- Runtime config accepts valid extension bindings and rejects invalid hook names, contracts, fail modes, local path traversal, and admin mount collisions.
- Bound control extension deny stops stack invocation.
- Bound control extension patch mutates request and response.
- Bound observer receives stack pre/post/failure/denied events.
- Observer fail-open continues and fail-closed blocks according to binding config.
- Admin surface manifest registers page/action/API routes under its mount.
- Admin API route from a surface runs through `pre_admin` and `post_admin`.
- Admin route failure emits `admin.action.failed`.

## Acceptance criteria

Sorx can consume generic extension binding metadata and expose/execute control, observer, and admin extension surfaces through the existing runtime-host contract with no product-specific extension branches.
