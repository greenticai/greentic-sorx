# PR 08 FoundationDB Provider Gap

PR 08 audited sibling `../greentic-sorla-providers` before adding any local
FoundationDB dependency.

The available `providers/provider-foundationdb` crate is not currently a direct
fit for SORX runtime execution:

- it is marked `publish = false`
- it exposes SoRLa provider-core event/projection traits
- its current backing behavior is a local/dev in-memory transactional model
- it does not expose a CRUD store trait compatible with SORX
  `CreateOp`/`GetOp`/`UpdateOp`/`QueryOp`/`DeleteOp`

SORX now has the adapter boundary needed for that integration:

- `StoreProviderKind::FoundationDb`
- `FoundationDbProviderConfig`
- `FoundationDbProviderAdapter`
- tenant/pack/version `ProviderNamespace` on every store operation
- `BindingResolver` for entity-to-provider/collection routing

The current SORX FoundationDB adapter deliberately returns
`provider_unavailable` instead of pretending to execute. The follow-up for
`greentic-sorla-providers` is to expose a SORX-compatible store adapter or a
shared provider trait that can map:

- create entity record
- get entity record by collection/id
- update entity record
- query records by simple JSON filter
- delete record by collection/id

The adapter must preserve SORX namespaces:

```text
sorx/{tenant_id}/{pack_name}/{pack_version}/{entity}/{id}
sorx/{tenant_id}/{pack_name}/{pack_version}/indexes/{entity}/{field}/{value}/{id}
```

If `greentic-sorla-providers` keeps its event/projection model as the canonical
FoundationDB abstraction, the missing piece is an entity-store projection layer
that implements those CRUD semantics over the event/projection provider.
