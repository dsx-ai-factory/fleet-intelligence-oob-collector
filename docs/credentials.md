# Credentials

The chart reads existing Kubernetes Secrets for BMC credentials and the Fleet
Intelligence bootstrap token. Create them directly as shown in
[Installation](installation.md#configure-targets-and-secrets), or synchronize
them from Vault, OpenBao, or another secrets manager with your chosen operator.
Apply provider resources separately from the collector chart.

| Credential | Kubernetes Secret key | Workload input |
| --- | --- | --- |
| Shared BMC credentials JSON | `credentials.json` | `/run/secrets/bmc/credentials.json` in the collector, NICo init container, and NICo runtime |
| Fleet Intelligence bootstrap token | `agent-token` | Collector `FLEETINT_AGENT_TOKEN` and bundled gateway `SAK_TOKEN` environment variables |

Credential values stay in the JSON file and token Secret. Helm values contain
inventory and Secret references only. The collector and bundled gateway use
the same bootstrap token, authorized for both enrollment flows.

## Provider prerequisites

| Backend | Server setup | Operator setup |
| --- | --- | --- |
| Vault | [Vault on Kubernetes](https://developer.hashicorp.com/vault/docs/deploy/kubernetes/helm/run) | [Install Vault Secrets Operator](https://developer.hashicorp.com/vault/docs/deploy/kubernetes/vso/installation) |
| OpenBao | [OpenBao on Kubernetes](https://openbao.org/docs/platform/k8s/helm/run/) | [Install External Secrets Operator](https://external-secrets.io/latest/introduction/getting-started/) |

Use a reachable server with trusted TLS, KV v2 mounts, Kubernetes
authentication, and the selected operator's CRDs installed. Choose one
operator for each destination Secret.

## 1. Store the credentials

Prepare the complete BMC JSON document using
[credentials.example.json](../credentials.example.json). Its `profiles` contain
shared credential fields, and its per-MAC `overrides` supply device-specific
fields. Each field resolves from its MAC override, then the target's selected
profile. See [Configuration](configuration.md#collector-toml) for the format.

Store the whole JSON document as a **string** field named `credentials` in KV
mount `bmc`, entry `collector`. The operator copies that string into the
Kubernetes Secret key `credentials.json`; it does not build a separate file
for each target or duplicate the credential-resolution logic.

Store the shared Fleet Intelligence bootstrap token as a non-empty string
field named `token` in KV mount `credentials`, entry `backend`. Use your
provider's CLI or UI to populate both entries; keep their values out of example
YAML and Helm values.

## 2. Authorize the operator's identity

Have your secrets administrator configure Kubernetes authentication, backend
TokenReview access, and a role with a read-only policy. Follow
[Vault Kubernetes auth](https://developer.hashicorp.com/vault/docs/auth/kubernetes)
or [OpenBao Kubernetes auth](https://openbao.org/docs/auth/kubernetes/).

The examples use these settings:

| Setting | Vault/VSO | OpenBao/ESO |
| --- | --- | --- |
| Auth mount | `kubernetes` | `kubernetes` |
| Backend role | `oob-credentials-reader` | `oob-credentials-reader` |
| Bound namespace | `fleet-intelligence` | `fleet-intelligence` |
| Bound ServiceAccount | `vault-reader` | `openbao-reader` |
| Token audience | `vault` | `vault` |

Grant `read` on `bmc/data/collector` and `credentials/data/backend`. These KV v2
policy paths include `data/`; the sync resources' source paths do not. The
collector and NICo do not authenticate to the secrets backend; the operator
uses the bound ServiceAccount to obtain a backend token.

## 3. Configure the connection and authentication

Create the namespace:

```bash
kubectl create namespace fleet-intelligence \
  --dry-run=client -o yaml | kubectl apply -f -
```

For Vault/VSO, edit [auth.yaml](../deployments/helm/examples/vault/auth.yaml) with
your server address and identity bindings. Set the `VaultConnection`'s
`caCertSecretRef` when using a private CA, as described in the
[VSO API reference](https://developer.hashicorp.com/vault/docs/deploy/kubernetes/vso/api-reference).
Apply the edited file:

```bash
kubectl apply -f /path/to/auth.yaml
kubectl get vaultconnection,vaultauth --namespace fleet-intelligence
```

For OpenBao/ESO, edit
[secret-stores.yaml](../deployments/helm/examples/openbao/secret-stores.yaml) with
your server, CA settings, and identity bindings. The `openbao-bmc` store selects
mount `bmc`; `openbao-tokens` selects mount `credentials`. ESO uses its `vault`
provider for OpenBao; see the
[OpenBao integration guide](https://external-secrets.io/latest/provider/openbao/).

```bash
kubectl apply -f /path/to/secret-stores.yaml
kubectl get secretstore --namespace fleet-intelligence
```

## 4. Synchronize Kubernetes Secrets

Edit the sync resources for your provider:

| Operator | BMC JSON | Bootstrap token | Resource |
| --- | --- | --- | --- |
| VSO | [bmc-secrets.yaml](../deployments/helm/examples/vault/bmc-secrets.yaml) | [token-secrets.yaml](../deployments/helm/examples/vault/token-secrets.yaml) | `VaultStaticSecret` |
| ESO | [bmc-secrets.yaml](../deployments/helm/examples/openbao/bmc-secrets.yaml) | [token-secrets.yaml](../deployments/helm/examples/openbao/token-secrets.yaml) | `ExternalSecret` |

Set the namespace, auth/store reference, source paths, destination names, and
refresh interval. Both providers produce these Secrets:

| Destination Secret | Key | Source mount/entry/field |
| --- | --- | --- |
| `fleet-intelligence-bmc-credentials` | `credentials.json` | `bmc` / `collector` / `credentials` |
| `fleet-intelligence-oob-collector-agent-token` | `agent-token` | `credentials` / `backend` / `token` |

The templates copy the source strings and reject missing or empty fields.
The collector validates the JSON credential schema when loading it. See
[VSO transformations](https://developer.hashicorp.com/vault/docs/deploy/kubernetes/vso/secret-transformation)
and [ESO templates](https://external-secrets.io/latest/guides/templating/) for
template behavior. The ESO examples use `external-secrets.io/v1`.

For VSO tokens, set `rolloutRestartTargets` to the collector and gateway
Deployment names for your release. For inventory-only deployments, keep only
the collector restart target.

```bash
kubectl apply -f /path/to/bmc-secrets.yaml
kubectl apply -f /path/to/token-secrets.yaml
```

## 5. Verify synchronization

Inspect status and events without decoding credential values:

```bash
# Vault/VSO
kubectl get vaultstaticsecret --namespace fleet-intelligence
kubectl describe vaultstaticsecret fleet-intelligence-bmc-credentials \
  --namespace fleet-intelligence

# OpenBao/ESO
kubectl get externalsecret --namespace fleet-intelligence
kubectl describe externalsecret fleet-intelligence-bmc-credentials \
  --namespace fleet-intelligence
```

Wait for both required Secrets to synchronize successfully before installing
the collector. A backend `403` after login indicates missing path permissions.

## 6. Deploy the collector

Use these Helm references with either provider:

```yaml
credentials:
  existingSecret: fleet-intelligence-bmc-credentials
  secretKey: credentials.json
backend:
  agentToken:
    existingSecret: fleet-intelligence-oob-collector-agent-token
    secretKey: agent-token
```

The same references are provided in
[credentials-values.yaml](../deployments/helm/examples/vault/credentials-values.yaml)
and [token-values.yaml](../deployments/helm/examples/vault/token-values.yaml).
Pass the edited files to the Helm command in [Installation](installation.md),
alongside your `inventory.targets` and backend URL.

Helm renders the collector TOML with the shared credentials path. NICo's init
container runs `render-nico-config` to generate generic static endpoints that
reference the same JSON file and each selected profile. Both runtimes read
credential values from the mounted JSON file.

## Rotation and ownership

Rotate the actual BMC password or bootstrap token through its issuing system,
then update the corresponding backend entry. The secrets operator synchronizes
the updated value into Kubernetes.

| Change | Runtime behavior |
| --- | --- |
| BMC JSON credentials | Collector reads the file each cycle; NICo reloads it at runtime. The generated NICo TOML still references the same file. |
| Shared bootstrap token | Restart the collector and bundled gateway so their environment variables use the updated Secret. |
| Target inventory | Run `helm upgrade` with the edited values; checksums roll both collector and NICo, and the init container regenerates NICo TOML. |

VSO polls at `refreshAfter`; its token example restarts both Deployments.
ESO polls at `refreshInterval`; use your platform's restart controller or
restart the enabled workloads after a bootstrap token update:

```bash
kubectl rollout restart deployment/fleet-intelligence-oob-collector \
  --namespace fleet-intelligence
kubectl rollout restart deployment/fleet-intelligence-oob-collector-otel-gateway \
  --namespace fleet-intelligence
```

Use your release's Deployment names and omit the gateway command when it is
disabled. Bootstrap token rotation and Fleet Intelligence runtime JWT renewal
are separate operations.

The sync examples give the operator ownership of destination Secrets. Manage
their retention and cleanup through the provider integration. Uninstalling the
collector Helm release leaves these separately applied resources in place.
