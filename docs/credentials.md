# Credentials

The Helm chart doesn't hold any secrets itself. It reads two Kubernetes Secrets that you create in advance:

| Secret contents | Key | Used by |
| --- | --- | --- |
| BMC credentials file (JSON) | `credentials.json` | The collector and NICo, mounted at `/run/secrets/bmc/credentials.json` |
| Fleet Intelligence bootstrap token | `agent-token` | The collector (`FLEETINT_AGENT_TOKEN`) and the bundled gateway (`SAK_TOKEN`) |

You can create these Secrets by hand, as shown in [Installation](installation.md#configure-targets-and-secrets). This page covers the alternative: keeping the values in Vault or OpenBao and letting a secrets operator copy them into Kubernetes. [Rotation and ownership](#rotation-and-ownership) applies either way.

Three separate logins are involved, and it helps to keep them apart:

- **The secrets operator** signs in to Vault or OpenBao with its Kubernetes ServiceAccount. The collector and NICo never talk to Vault or OpenBao.
- **The collector and NICo** sign in to BMCs with the usernames and passwords in the credentials file.
- **The collector and gateway** each exchange the bootstrap token for their own Fleet Intelligence session. Get the bootstrap token from your Fleet Intelligence administrator. A Vault or OpenBao token can't be used in its place.

## Before you start

Pick one operator for each Secret. The examples pair Vault with the Vault Secrets Operator (VSO) and OpenBao with the External Secrets Operator (ESO):

| Secrets manager | Server setup | Operator setup |
| --- | --- | --- |
| Vault | [Vault on Kubernetes](https://developer.hashicorp.com/vault/docs/deploy/kubernetes/helm/run) | [Install Vault Secrets Operator](https://developer.hashicorp.com/vault/docs/deploy/kubernetes/vso/installation) |
| OpenBao | [OpenBao on Kubernetes](https://openbao.org/docs/platform/k8s/helm/run/) | [Install External Secrets Operator](https://external-secrets.io/latest/introduction/getting-started/) |

You need a server the cluster can reach over trusted TLS, KV version 2 mounts, Kubernetes authentication, and the operator's CRDs installed.

The commands on this page use the namespace in `$NAMESPACE`. Use the same namespace you install the collector into, and set it again if you continue in a new terminal:

```bash
export NAMESPACE=fleet-intelligence
```

The example manifests can't read shell variables, so they name the namespace `fleet-intelligence` directly. If you use a different namespace, change it in each manifest you apply.

## 1. Store the credentials

Prepare your BMC credentials file from [credentials.example.json](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/credentials.example.json). [Configuration](configuration.md#bmc-credentials-file) explains its profiles and overrides.

Store two entries:

| What | KV mount | Entry | Field |
| --- | --- | --- | --- |
| The whole credentials file, as one string | `bmc` | `collector` | `credentials` |
| The bootstrap token | `credentials` | `backend` | `token` |

With the provider CLI signed in and both mounts enabled, load them straight from files:

```bash
SECRETS_CLI=vault # Use bao for OpenBao.
"$SECRETS_CLI" kv put -mount=bmc collector \
  credentials=@/secure/path/to/credentials.json
"$SECRETS_CLI" kv put -mount=credentials backend \
  token=@/secure/path/to/agent-token
```

The `@` form stores each file's contents as a single string, which is what the sync templates expect. Store the whole credentials file in one field; the operator copies it into Kubernetes unchanged. See the [Vault](https://developer.hashicorp.com/vault/docs/commands/kv/put) and [OpenBao](https://openbao.org/docs/commands/kv/put/) CLI references.

## 2. Let the operator read them

Your secrets administrator sets up Kubernetes authentication (including TokenReview access) by following the [Vault Kubernetes auth](https://developer.hashicorp.com/vault/docs/auth/kubernetes) or [OpenBao Kubernetes auth](https://openbao.org/docs/auth/kubernetes/) guide. You then need a read-only policy and a role for the operator. The examples use these names:

| Setting | Vault/VSO | OpenBao/ESO |
| --- | --- | --- |
| Auth mount | `kubernetes` | `kubernetes` |
| Role | `oob-credentials-reader` | `oob-credentials-reader` |
| Namespace | `$NAMESPACE` | `$NAMESPACE` |
| ServiceAccount | `vault-reader` | `openbao-reader` |
| Token audience | `vault` | `vault` |

Create the policy and role. For OpenBao, set `SECRETS_CLI=bao` and `SECRETS_READER_SERVICE_ACCOUNT=openbao-reader`. If you change the audience, change it here and in the manifests in the next steps.

```bash
SECRETS_CLI=vault
SECRETS_READER_SERVICE_ACCOUNT=vault-reader

"$SECRETS_CLI" policy write oob-credentials-reader - <<'EOF'
path "bmc/data/collector" {
  capabilities = ["read"]
}
path "credentials/data/backend" {
  capabilities = ["read"]
}
EOF

"$SECRETS_CLI" write auth/kubernetes/role/oob-credentials-reader \
  bound_service_account_names="$SECRETS_READER_SERVICE_ACCOUNT" \
  bound_service_account_namespaces="$NAMESPACE" \
  audience=vault \
  policies=oob-credentials-reader \
  ttl=1h
```

Policy paths include `data/` (for example `bmc/data/collector`), because that is how KV version 2 names them. The sync manifests in step 4 leave it out.

## 3. Connect the operator to your server

Create the namespace:

```bash
kubectl create namespace "$NAMESPACE" \
  --dry-run=client -o yaml | kubectl apply -f -
```

**Vault/VSO:** edit [auth.yaml](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/helm/examples/vault/auth.yaml) with your server address and identity settings. If your server uses a private CA, set `caCertSecretRef` on the `VaultConnection` (see the [VSO API reference](https://developer.hashicorp.com/vault/docs/deploy/kubernetes/vso/api-reference)). Then apply it:

```bash
kubectl apply -f /path/to/auth.yaml
kubectl get vaultconnection,vaultauth --namespace "$NAMESPACE"
```

**OpenBao/ESO:** edit [secret-stores.yaml](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/helm/examples/openbao/secret-stores.yaml) with your server, CA, and identity settings. It defines two stores: `openbao-bmc` for the `bmc` mount and `openbao-tokens` for the `credentials` mount. ESO talks to OpenBao through its `vault` provider; see the [OpenBao integration guide](https://external-secrets.io/latest/provider/openbao/).

```bash
kubectl apply -f /path/to/secret-stores.yaml
kubectl get secretstore --namespace "$NAMESPACE"
```

## 4. Sync the Secrets into Kubernetes

Edit the sync manifests for your operator. Set the namespace, auth or store reference, source paths, Secret names, and refresh interval.

| Operator | Credentials file | Bootstrap token | Resource type |
| --- | --- | --- | --- |
| VSO | [bmc-secrets.yaml](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/helm/examples/vault/bmc-secrets.yaml) | [token-secrets.yaml](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/helm/examples/vault/token-secrets.yaml) | `VaultStaticSecret` |
| ESO | [bmc-secrets.yaml](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/helm/examples/openbao/bmc-secrets.yaml) | [token-secrets.yaml](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/helm/examples/openbao/token-secrets.yaml) | `ExternalSecret` (`external-secrets.io/v1`) |

For VSO, also set `rolloutRestartTargets` in the token manifest to your release's collector and gateway Deployments, so they restart when the token changes. If you don't run the gateway, list only the collector.

```bash
kubectl apply -f /path/to/bmc-secrets.yaml
kubectl apply -f /path/to/token-secrets.yaml
```

Either operator produces the same two Secrets:

| Secret | Key | Copied from (mount / entry / field) |
| --- | --- | --- |
| `fleet-intelligence-bmc-credentials` | `credentials.json` | `bmc` / `collector` / `credentials` |
| `fleet-intelligence-oob-collector-agent-token` | `agent-token` | `credentials` / `backend` / `token` |

The credentials file is copied exactly, so passwords keep any leading or trailing spaces. The token is trimmed of surrounding whitespace and newlines, and a missing or blank token stops the sync instead of writing an empty Secret. To change this behavior, see [VSO transformations](https://developer.hashicorp.com/vault/docs/deploy/kubernetes/vso/secret-transformation) or [ESO templates](https://external-secrets.io/latest/guides/templating/).

## 5. Check that the sync worked

Check status and events. None of these commands print secret values.

```bash
# Vault/VSO
kubectl get vaultstaticsecret --namespace "$NAMESPACE"
kubectl describe vaultstaticsecret fleet-intelligence-bmc-credentials \
  fleet-intelligence-oob-collector-agent-token \
  --namespace "$NAMESPACE"

# OpenBao/ESO
kubectl get externalsecret --namespace "$NAMESPACE"
kubectl describe externalsecret fleet-intelligence-bmc-credentials \
  fleet-intelligence-oob-collector-agent-token \
  --namespace "$NAMESPACE"

kubectl wait --for=condition=Ready \
  externalsecret/fleet-intelligence-bmc-credentials \
  externalsecret/fleet-intelligence-oob-collector-agent-token \
  --namespace "$NAMESPACE" --timeout=120s
```

Then confirm each Secret has the right key:

```bash
kubectl get secret fleet-intelligence-bmc-credentials \
  fleet-intelligence-oob-collector-agent-token \
  --namespace "$NAMESPACE" \
  -o go-template='{{range .items}}{{.metadata.name}}:{{range $key, $value := .data}} {{$key}}{{end}}{{"\n"}}{{end}}'
```

You should see `credentials.json` in `fleet-intelligence-bmc-credentials` and `agent-token` in `fleet-intelligence-oob-collector-agent-token`. Wait until both are present before installing the collector.

| Problem | What to check |
| --- | --- |
| The operator can't sign in | The role's ServiceAccount, namespace, and audience |
| Sign-in works, but reading returns `403` | The policy paths and capabilities |

## 6. Install the collector

Point the chart at the synced Secrets. The same settings are in [credentials-values.yaml](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/helm/examples/vault/credentials-values.yaml) and [token-values.yaml](https://github.com/dsx-ai-factory/fleet-intelligence-oob-collector/blob/main/deployments/helm/examples/vault/token-values.yaml), and they work with either operator:

```yaml
credentials:
  existingSecret: fleet-intelligence-bmc-credentials
  secretKey: credentials.json
backend:
  agentToken:
    existingSecret: fleet-intelligence-oob-collector-agent-token
    secretKey: agent-token
```

Install with these values and your targets, following [Installation](installation.md#install). A successful sync doesn't prove the credentials work, so finish with [the checks in Installation](installation.md#verify).

## Rotation and ownership

To rotate a BMC password or the bootstrap token, change it at its source (the BMC, or your Fleet Intelligence administrator), then update the entry in your secrets manager or the Kubernetes Secret. The operator copies the new value into Kubernetes on its next refresh: `refreshAfter` for VSO, `refreshInterval` for ESO.

What happens next depends on what changed:

| Change | What you need to do |
| --- | --- |
| BMC passwords in the credentials file | Nothing. The collector reads the file at the start of each collection cycle, and NICo picks up changes on its own. If a BMC rejects a password during attestation, the collector reloads the file once and retries. |
| Bootstrap token | Restart the collector and gateway, because they read the token only at startup. The VSO token example does this for you. With ESO or a hand-made Secret, restart them yourself (below). |
| Targets or other Helm values | Run `helm upgrade`. The chart restarts the workloads whose configuration changed: target changes restart the collector and NICo, and gateway settings restart the gateway. |

Restart after a token change:

```bash
kubectl rollout restart deployment/fleet-intelligence-oob-collector \
  --namespace "$NAMESPACE"
kubectl rollout restart deployment/fleet-intelligence-oob-collector-otel-gateway \
  --namespace "$NAMESPACE"
```

Use your release's Deployment names, and skip the gateway if it's disabled. You don't need to do anything for the short-lived Fleet Intelligence sessions; the collector and gateway renew those themselves.

The operator owns the Secrets it creates, so clean them up through the operator. Uninstalling the collector chart leaves the operator resources and Secrets in place.
