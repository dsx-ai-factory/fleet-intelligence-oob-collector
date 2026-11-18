# Discover targets through NICo REST

Use NICo REST to discover compute-host BMC IPs, MAC addresses, and optional hostnames in one site. The collector authenticates as a service through the DSX Starfleet SSA OAuth2 client-credentials flow.

This integration supplies targets to the existing collector. Redfish authentication and collection are unchanged. NICo inventory and attestation records are not imported.

## Prepare service access

Follow the DSX **Starfleet SSA Client Provisioning** guide to provision a dedicated SSA client with the `carbide` scope and NICo provider/site permissions. The platform stores and rotates the client credentials in Vault under `services/dsx/clients/<component>/issue/creds` in the `dgxc-dsx` namespace.

Use the platform's Vault Secrets Operator (VSO) configuration to sync those credentials into a Secret in the collector namespace. The Secret must contain `client_id` and `client_secret` keys. Vault's `secret` field maps to the Kubernetes `client_secret` key. The [SSA credentials example](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/helm/examples/nico-discovery/ssa-credentials.yaml) follows the DSX `kv-v1` and transformation configuration; have your platform administrator wire the reader service account and VaultAuth to your provisioned client.

Follow [Kubernetes installation](installation.md) to prepare the namespace, FI enrollment Secret, and existing [BMC credentials Secret](configuration.md#bmc-credentials-file). Replace the static target values with the source settings below.

## Service authentication

The chart mounts the SSA credential Secret read-only as `/run/secrets/nico-ssa/client_id` and `/run/secrets/nico-ssa/client_secret`, without `subPath`. VSO updates the Secret when Vault rotates credentials, and Kubernetes projects those updates into the mounted directory.

On each discovery, the collector rereads both files and uses the Rust `oauth2` client to request a fresh access token from the configured SSA token endpoint. It sends HTTP Basic client authentication with `grant_type=client_credentials` and `scope=carbide`, then uses the returned bearer token for NICo requests. The token stays in memory for that listing. There is no pre-issued access-token file to maintain and no client secret in Helm values or the ConfigMap. Rotated credentials are used on the next discovery attempt without restarting the pod.

Both endpoints require trusted HTTPS certificates and reject redirects. A whole-discovery timeout covers credential loading, the token exchange, and NICo pagination. Failed SSA authentication prevents the NICo query.

## Configure Kubernetes

Add these settings to your Helm values and omit `inventory.targets`:

```yaml
endpointSource:
  type: nico
  nico:
    apiUrl: https://nico.example.com/v2
    org: example-org
    siteId: 00000000-0000-0000-0000-000000000001
    apiName: nico
    tokenUrl: https://ssa.example.com/token
    credentialsSecretName: fi-nico-ssa-credentials
    credentialsProfile: bmc
    timeoutSeconds: 30

inventory:
  computeZone: us-west-dc1
  nodeGroup: training
  labels:
    site: dc1

nicoHardwareHealth:
  enabled: false

otelGateway:
  enabled: false
```

Keep your normal FI backend, enrollment Secret, and BMC credentials Secret settings. The chart deploys only the collector with this configuration. Use the existing site's NICo health and telemetry services separately; their configuration is unchanged. Install or upgrade with the command in the [installation guide](installation.md#install).

| Helm value | Meaning |
| --- | --- |
| `endpointSource.type` | `static` by default; select `nico` for REST discovery |
| `endpointSource.nico.apiUrl` | HTTPS REST prefix including `/v2`; omit organization and API-name segments |
| `endpointSource.nico.org` | NICo organization |
| `endpointSource.nico.siteId` | Site to discover |
| `endpointSource.nico.apiName` | `nico` by default; use `carbide` for deployments with the legacy route |
| `endpointSource.nico.tokenUrl` | HTTPS token endpoint from the DSX SSA provisioning guide; replace the example URL |
| `endpointSource.nico.credentialsSecretName` | VSO-managed Secret in the collector namespace with `client_id` and `client_secret` keys |
| `endpointSource.nico.credentialsProfile` | Optional existing BMC credential profile; per-MAC overrides also apply |
| `endpointSource.nico.timeoutSeconds` | Whole discovery deadline, including the SSA exchange, all pages, and one retry for inconsistent pagination; default `30`, allowed `1`–`3600` |

The chart writes these settings into `[endpoint_source]` in the collector TOML, with `client_id_file` and `client_secret_file` pointing to the mounted files. For an explicit TOML example, see [config.nico.example.toml](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/config.nico.example.toml).

## Verify discovery

List targets from an installed collector pod without querying BMCs or reading their passwords:

```bash
kubectl exec --namespace "$NAMESPACE" deployment/fleet-intelligence-oob-collector -- \
  /bmc-collector discover --config /etc/fleet-intelligence/config.toml
```

The JSON output includes IP, MAC, hostname, and kind. Treat it as infrastructure information. The source supplies `compute` targets on HTTPS port 443. NICo machine labels are ignored; configure FI placement and labels through `inventory.computeZone`, `inventory.nodeGroup`, and `inventory.labels`.

The runtime discovers targets when each collection cycle loads its configuration. Inventory and attestation share that target list. There is no separate refresh loop or cached target fallback. A discovery failure aborts the cycle; at startup it exits the process, allowing Kubernetes to restart it.

## Troubleshoot discovery

| Problem | What to check |
| --- | --- |
| SSA OAuth2 exchange failure | VSO synchronization, both credential files, token endpoint, CA trust, and the client's `carbide` scope |
| NICo HTTP `401` or `403` | SSA client scope and NICo provider/site permissions |
| HTTP `404` | REST prefix and `apiName` for the deployment |
| Skipped machines | Warnings report missing or invalid BMC metadata; check onboarding and metadata permissions |
| No targets | Selected site and host visibility; empty usable listings are rejected |
| Inconsistent pages | Membership changed during pagination; the collector retries the listing once within the original deadline |
| Timeout | Connectivity, CA trust, site size, and the whole-discovery deadline |
| Unknown BMC profile | Existing BMC credentials Secret and `credentialsProfile`; this is separate from REST authentication |

Discovery requests non-missing machines in `Ready`, `InUse`, `Initializing`, `Error`, `Maintenance`, `Reset`, and `Unknown` states. Decommissioning and decommissioned machines are excluded. Machines without valid BMC IP/MAC metadata are skipped; warnings identify the first 20 and report total skipped and usable counts. Duplicate BMC IPs or MACs reject the listing.

Every page must succeed before targets are published. Requests use 100 machines per page, with at most 1,000 pages and 8 MiB per response page. Changed totals, incomplete pages, or repeated machine IDs trigger one retry from page 1. These checks cannot provide an atomic view of a changing site.
