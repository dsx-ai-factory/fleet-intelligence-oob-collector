# Discover targets through NICo REST

Use NICo REST to discover compute-host BMC IPs, MAC addresses, and optional hostnames in one site. Kubernetes mounts your NICo access token from a Secret into a file. The collector reads that file each time it discovers targets.

This integration supplies targets to the existing collector. Redfish authentication and collection are unchanged. NICo inventory and attestation records are not imported.

## Prepare access

You need an HTTPS NICo REST endpoint trusted by the collector's system CA store, an organization, a site ID, and a bearer access token with permission to read machine BMC metadata. Confirm the `carbide` scope and provider/site permissions with your NICo administrator. The file must contain an access token, rather than an SSA client secret. Arrange token issuance and refresh outside the collector.

Follow [Kubernetes installation](installation.md) to prepare the namespace, FI enrollment Secret, and existing [BMC credentials Secret](configuration.md#bmc-credentials-file). Replace the static target values with the source settings below.

## Mount the token

Save the access token in a protected local file and create a Secret in the collector namespace:

```bash
kubectl create secret generic fi-nico-access-token \
  --namespace "$NAMESPACE" \
  --from-file=access-token=/secure/path/to/nico-access-token
```

Keep this Secret updated when the token rotates. The chart mounts its selected key read-only at `/run/secrets/nico-rest/access-token`, without `subPath`, so Kubernetes can project Secret updates. The collector rereads the file on each discovery. No token value belongs in Helm values or the ConfigMap.

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
    tokenSecretName: fi-nico-access-token
    tokenSecretKey: access-token
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
| `endpointSource.nico.tokenSecretName` | Existing Secret in the collector namespace |
| `endpointSource.nico.tokenSecretKey` | Secret key containing the access token; default `access-token` |
| `endpointSource.nico.credentialsProfile` | Optional existing BMC credential profile; per-MAC overrides also apply |
| `endpointSource.nico.timeoutSeconds` | Whole discovery deadline, including all pages and one retry for inconsistent pagination; default `30`, allowed `1`–`3600` |

The chart writes these settings into `[endpoint_source]` in the collector TOML, with `token_file` pointing to the mounted file. For an explicit TOML example, see [config.nico.example.toml](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/config.nico.example.toml).

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
| HTTP `401` or `403` | Token expiry, `carbide` scope, and provider/site permissions |
| HTTP `404` | REST prefix and `apiName` for the deployment |
| Skipped machines | Warnings report missing or invalid BMC metadata; check onboarding and metadata permissions |
| No targets | Selected site and host visibility; empty usable listings are rejected |
| Inconsistent pages | Membership changed during pagination; the collector retries the listing once within the original deadline |
| Timeout | Connectivity, CA trust, site size, and the whole-discovery deadline |
| Unknown BMC profile | Existing BMC credentials Secret and `credentialsProfile`; this is separate from REST authentication |

Discovery requests non-missing machines in `Ready`, `InUse`, `Initializing`, `Error`, `Maintenance`, `Reset`, and `Unknown` states. Decommissioning and decommissioned machines are excluded. Machines without valid BMC IP/MAC metadata are skipped; warnings identify the first 20 and report total skipped and usable counts. Duplicate BMC IPs or MACs reject the listing.

Every page must succeed before targets are published. Requests use 100 machines per page, with at most 1,000 pages and 8 MiB per response page. Changed totals, incomplete pages, or repeated machine IDs trigger one retry from page 1. These checks cannot provide an atomic view of a changing site.
