# Configuration snapshot

CI renders `templates/configmap.yaml` with release name `test` and
`test-values.yaml`, then compares the complete output with
`expected-configmap.yaml`. The snapshot covers the generated collector TOML.
NICo TOML is generated at startup by `bmc-collector render-nico-config`.
It is generated output; preserve its whitespace.

After an intentional configuration change, regenerate the snapshot from the
repository root and review the diff before committing:

```bash
helm template test deployments/helm/fleet-intelligence-oob-collector \
  --values deployments/helm/fleet-intelligence-oob-collector/ci/test-values.yaml \
  --show-only templates/configmap.yaml \
  > deployments/helm/fleet-intelligence-oob-collector/ci/expected-configmap.yaml
```
