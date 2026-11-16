# Endpoint sources

Keep static BMC endpoints in `targets.json` and shared collection settings in `config.toml`. This is the recommended static configuration.

## Select a static JSON file

Keep your zone, node group, global labels, NICo health controls, and credential-file reference in `config.toml`:

```toml
compute_zone = "us-west-dc1"
node_group = "training"
labels = { site = "dc1" }

[credentials]
file = "credentials.json"

[endpoint_source]
type = "static"
file = "targets.json"
```

Create `targets.json` as an array of endpoint records:

```json
[
  {
    "kind": "compute",
    "name": "node-01",
    "ip": "192.0.2.10",
    "mac": "02:00:00:00:00:01",
    "credentials_profile": "bmc"
  }
]
```

Each record requires `ip` and `mac`. Optional fields are `kind`, `name`, `port`, `credentials_profile`, `labels`, `compute_zone`, and `node_group`. Omitted `kind` defaults to `unknown`. Endpoint labels and placement override the shared defaults. Credentials come from the [separate credentials file](credentials.md); the JSON target list contains profile references rather than passwords.

Relative target and credential paths resolve from the directory containing `config.toml`. Protect all three files from other users. Replace the target file atomically to avoid a collection cycle reading a partially written file.

Use this configuration with `inventory`, `run`, or `render-nico-config`. The collector loads one target snapshot per collection cycle and shares it between inventory and attestation. Updated targets take effect on the next cycle; there is no separate refresh loop. If you use `render-nico-config` to configure NICo hardware health, regenerate that file and restart NICo after changing targets.

An unreadable file, invalid JSON, empty array, invalid endpoint, duplicate MAC, or unresolved credential causes configuration loading to fail. Errors omit JSON contents and passwords.

## Deprecated inline targets

Inline `[[targets]]` entries in `config.toml` are deprecated and remain supported for compatibility. For new or updated configurations, move those endpoint records to `targets.json` and select the static file source above. Remove inline entries when adding `endpoint_source`; combining both forms is rejected. Helm-generated configuration and the installed package template still use the compatibility format.
