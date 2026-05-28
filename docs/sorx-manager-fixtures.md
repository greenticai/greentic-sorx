# Sorx Manager Fixtures

Manager fixtures are domain-neutral. New manager tests should use synthetic records such as `RecordAlpha` and `RecordBeta`, not landlord/tenant, finance, healthcare, ticketing, or other business-domain assumptions.

Fixture location:

```text
crates/greentic-sorx-cli/tests/e2e/fixtures/manager/
```

The first checked fixture is:

```text
manager/basic-records/agent-gateway.json
```

It proves that generic records produce navigation, fields, create cards, approval-required action state, and manager graph metadata without special-casing a domain.

Pack-shaped fixtures should still follow current `.gtpack` required entries:

```text
pack.cbor
assets/sorla/model.cbor
assets/sorla/agent-gateway.json
assets/sorx/start.schema.json
```

Pack-embedded validation fixtures should use `assets/sorx/validation-suite.json` and referenced JSON fixture entries, matching [validation suites](validation-suites.md).
