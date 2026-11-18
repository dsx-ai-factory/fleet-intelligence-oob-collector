# Discover targets through NICo REST

Use NICo REST to discover compute-host BMC IPs, MAC addresses, and optional hostnames in one site. The collector authenticates with OAuth2 client credentials through the identity provider configured for your NICo deployment.

This integration supplies targets to the existing collector. Redfish authentication and collection are unchanged. NICo inventory and attestation records are not imported.

## Prepare service access

Ask your platform administrator to register an OAuth2 client allowed to read NICo machine BMC metadata. Obtain the HTTPS token endpoint and required OAuth2 scopes. NICo supports configurable identity providers and scope requirements; see its [issuer configuration guide](https://github.com/dsx-ai-factory/infra-controller/blob/main/rest-api/auth/README.md).

Create or sync a Secret in the collector namespace containing `client_id` and `client_secret` keys. Your platform manages credential provisioning and rotation. Keep the values out of Helm values and the collector ConfigMap.

Follow [Kubernetes installation](installation.md) to prepare the namespace, FI enrollment Secret, and existing [BMC credentials Secret](configuration.md#bmc-credentials-file). Replace the static target values with the source settings below.

## Service authentication

The chart mounts the OAuth2 credential Secret read-only as `/run/secrets/nico-oauth2/client_id` and `/run/secrets/nico-oauth2/client_secret`, without `subPath`. Kubernetes projects Secret updates into the mounted directory.

On each discovery, the collector rereads both files and uses the Rust `oauth2` client to request a fresh access token. It uses HTTP Basic client authentication and `grant_type=client_credentials`, requesting the configured `scopes`. An empty scope list omits the OAuth2 scope parameter. The returned bearer token stays in memory for that listing. Rotated credentials are used on the next discovery attempt without restarting the pod.

Both endpoints require trusted HTTPS certificates and reject redirects. A whole-discovery timeout covers credential loading, the token exchange, and NICo pagination. Failed OAuth2 authentication prevents the NICo query.

## Configure Kubernetes

Add these settings to your Helm values and omit `inventory.targets`:

```yaml
endpointSource:
  type: nico
  nico:
    apiUrl: https://nico.example.com/v2
    org: example-org
    siteId: 00000000-0000-0000-0000-000000000001
    tokenUrl: https://idp.example.com/token
    oauth2CredentialsSecretName: fi-nico-oauth2-credentials
    scopes: [nico] # Example; use scopes configured by your IdP administrator.
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
| `endpointSource.nico.apiUrl` | HTTPS REST prefix ending in `/v2` |
| `endpointSource.nico.org` | NICo organization |
| `endpointSource.nico.siteId` | Site to discover |
| `endpointSource.nico.tokenUrl` | HTTPS OAuth2 token endpoint from your identity-provider administrator |
| `endpointSource.nico.oauth2CredentialsSecretName` | Secret in the collector namespace with `client_id` and `client_secret` keys |
| `endpointSource.nico.scopes` | OAuth2 scope names to request; default `[]` omits the scope parameter |
| `endpointSource.nico.credentialsProfile` | Optional existing BMC credential profile; per-MAC overrides also apply |
| `endpointSource.nico.timeoutSeconds` | Whole discovery deadline, including the OAuth2 exchange, all pages, and one retry for inconsistent pagination; default `30`, allowed `1`–`3600` |

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
| OAuth2 exchange failure | Secret synchronization, both credential files, token endpoint, CA trust, and configured scopes |
| NICo HTTP `401` or `403` | OAuth2 client scopes and NICo provider/site permissions |
| HTTP `404` | REST prefix and organization; machine discovery uses `/v2/org/{org}/nico/machine` |
| Skipped machines | Warnings report missing or invalid BMC metadata; check onboarding and metadata permissions |
| No targets | Selected site and host visibility; empty usable listings are rejected |
| Inconsistent pages | Membership changed during pagination; the collector retries the listing once within the original deadline |
| Timeout | Connectivity, CA trust, site size, and the whole-discovery deadline |
| Unknown BMC profile | Existing BMC credentials Secret and `credentialsProfile`; this is separate from REST authentication |

Discovery requests non-missing machines in `Ready`, `InUse`, `Initializing`, `Error`, `Maintenance`, `Reset`, and `Unknown` states. Decommissioning and decommissioned machines are excluded. Machines without valid BMC IP/MAC metadata are skipped; warnings identify the first 20 and report total skipped and usable counts. Duplicate BMC IPs or MACs reject the listing.

Every page must succeed before targets are published. Requests use 100 machines per page, with at most 1,000 pages and 8 MiB per response page. Changed totals, incomplete pages, or repeated machine IDs trigger one retry from page 1. These checks cannot provide an atomic view of a changing site.
