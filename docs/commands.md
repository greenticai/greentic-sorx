# Commands

```bash
greentic-sorx doctor landlord.gtpack --json
greentic-sorx inspect landlord.gtpack
greentic-sorx routes landlord.gtpack --json
greentic-sorx routes --deployment <deployment-id> --json
greentic-sorx mcp-tools landlord.gtpack
greentic-sorx graph concepts landlord.gtpack --json
greentic-sorx graph relationships landlord.gtpack --json
greentic-sorx graph paths landlord.gtpack --from Tenant --to Payment --json
greentic-sorx graph neighbors landlord.gtpack --entity-type Tenant --entity-id tenant-1 --depth 2 --json
greentic-sorx graph explain landlord.gtpack --from Tenant --to Payment --json
greentic-sorx evidence query landlord.gtpack --answers landlord.answers.json --query "lease status" --entity-type Tenant --entity-id tenant-1 --max-depth 2 --json
greentic-sorx deployments list
greentic-sorx deployments inspect <deployment-id>
greentic-sorx deployments create --pack landlord.gtpack --tenant acme --sor landlord --environment production --api-version v1 --base-path /sorx/acme/landlord/v1 --visibility private
greentic-sorx deployments validate <deployment-id>
greentic-sorx deployments activate <deployment-id> --private
greentic-sorx deployments promote <deployment-id> --public
greentic-sorx deployments promote <deployment-id> --alias preview
greentic-sorx deployments promote <deployment-id> --alias latest --public
greentic-sorx deployments rollback --tenant acme --sor landlord --alias latest --to <previous-deployment-id>
greentic-sorx deployments retire-old --tenant acme --sor landlord --keep 3
greentic-sorx deployments public-routes
greentic-sorx deployments promotion-status <deployment-id>
greentic-sorx deployments retire <deployment-id>
greentic-sorx aliases set --tenant acme --sor landlord --alias stable --target <deployment-id>
greentic-sorx aliases list --tenant acme
greentic-sorx webhook verify-fixture fixtures/github-ghcr-published.json
greentic-sorx webhook replay --fixture fixtures/github-ghcr-published.json --signature sha256:<hmac>
greentic-sorx validate landlord.gtpack --answers landlord.answers.json --provider-mode in-memory --json
greentic-sorx validation report <deployment-id>
greentic-sorx start landlord.gtpack --schema --json
greentic-sorx start landlord.gtpack --answers landlord.answers.json
greentic-sorx start landlord.gtpack --answers landlord.answers.json --dry-run --json
greentic-sorx start landlord.gtpack --answers landlord.answers.json --emit-answers
greentic-sorx run landlord.gtpack --answers landlord.answers.json
greentic-sorx mcp start landlord.gtpack --answers landlord.answers.json
```

Stable exit codes:

| Code | Meaning |
| ---: | --- |
| 0 | Success |
| 1 | Generic failure |
| 2 | CLI usage error |
| 3 | Pack validation failure |
| 4 | Startup answer validation failure |
| 5 | Provider resolution failure |
| 6 | Runtime startup failure |
| 7 | Future policy denial dry-run code |

`run` is currently an alias for the HTTP `start --answers` path. `mcp start`
validates answers and emits an adapter runtime plan; full MCP server transport
is still future work.

`inspect` includes a `business_actions` summary when a pack contains
`assets/sorla/business-actions.json`, including action count, lock presence,
hash validity, and execution-target validity.

`inspect` includes an `ontology` summary when a pack contains
`assets/sorla/ontology.graph.json`, including the graph schema, concept count,
relationship count, and whether `assets/sorla/retrieval-bindings.json` is
present.

`start --dry-run --json` includes `provider_compatibility` for ontology-enabled
packs. Packs without ontology report a passed compatibility section with no
bindings or issues.

Registry commands write to `--registry <path>`, `SORX_REGISTRY_PATH`, or the
default user config path.
