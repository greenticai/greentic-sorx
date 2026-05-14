# Ontology Security

SORX validates ontology-enabled packs and startup answers before using them at
runtime.

Security requirements enforced by this repo:

- ontology graph and retrieval binding assets are checked for secret-like values
  and absolute local paths
- startup answers reject inline secret-like values such as API keys, passwords,
  tokens, and private keys unless the value is a reference
- direct provider configuration is allowed only for local or test environments;
  production-style answers must use `config_ref`
- audit and explain payloads redact secret-like keys before output
- ontology policy can deny restricted relationship traversal and evidence
  retrieval that requires approval
- public route promotion is blocked unless validation gates allow exposure

SORX should store references to external credentials, not credential material
itself. Accepted reference forms include `secret:`, `secrets.`, `ref:`,
`vault:`, and environment substitutions.
