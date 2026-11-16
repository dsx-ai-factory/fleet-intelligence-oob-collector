# Endpoint sources

You can keep BMC endpoints in a separate JSON file while keeping collection settings in `config.toml`. Select a static file source when a separate process maintains your target list or you want to manage endpoints independently.

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

## Inline TOML targets

Existing configurations with `[[targets]]` continue to work without an `endpoint_source` section. Choose one target definition per configuration: combining `endpoint_source` with nonempty `[[targets]]` is rejected. Helm and the bundled Linux package configuration continue to use inline targets.
