{{/*
Copyright (c) 2025-2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
SPDX-License-Identifier: Apache-2.0
*/}}
{{/* Keep generated TOML limited to validated inventory and NICo values.
Optional strings are rendered only when nonblank after trimming, as chart
validation checks them, so a whitespace-only value is omitted, never empty. */}}
{{- define "fleet-intelligence-oob-collector.tomlLabels" -}}
{{- $entries := list -}}
{{- range $name, $value := . -}}
{{- $entries = append $entries (printf "%s = %s" (toJson $name) (toJson $value)) -}}
{{- end -}}
{{- printf "labels = { %s }" (join ", " $entries) -}}
{{- end -}}

{{- define "fleet-intelligence-oob-collector.collectorConfig" -}}
{{ with trim (.Values.inventory.computeZone | default "") }}compute_zone = {{ toJson . }}
{{ end }}{{ with trim (.Values.inventory.nodeGroup | default "") }}node_group = {{ toJson . }}
{{ end }}{{ include "fleet-intelligence-oob-collector.tomlLabels" .Values.inventory.labels }}

{{- if eq (default dict .Values.endpointSource).type "nico" }}
[endpoint_source]
type = "nico"
api_url = {{ required "endpointSource.nico.apiUrl is required" .Values.endpointSource.nico.apiUrl | toJson }}
org = {{ required "endpointSource.nico.org is required" .Values.endpointSource.nico.org | toJson }}
token_url = {{ required "endpointSource.nico.tokenUrl is required" .Values.endpointSource.nico.tokenUrl | toJson }}
client_id_file = "/run/secrets/nico-oauth2/client_id"
client_secret_file = "/run/secrets/nico-oauth2/client_secret"
scopes = {{ .Values.endpointSource.nico.scopes | default list | toJson }}
timeout_seconds = {{ .Values.endpointSource.nico.timeoutSeconds | default 30 }}

[bmc_proxy]
url = {{ required "endpointSource.nico.bmcProxy.url is required" .Values.endpointSource.nico.bmcProxy.url | toJson }}
ca_cert_path = "/run/secrets/nico-bmc-proxy/ca.crt"
client_cert_path = "/run/secrets/nico-bmc-proxy/tls.crt"
client_key_path = "/run/secrets/nico-bmc-proxy/tls.key"
{{- else }}

[credentials]
file = "/run/secrets/bmc/credentials.json"
{{- end }}
{{- $nico := dict -}}
{{- $config := default dict .Values.nicoHardwareHealth.config -}}
{{- range $key := list
    "sensorFetchIntervalSeconds"
    "bmcRequestConcurrency"
    "endpointDiscoveryIntervalSeconds"
    "discoveryRefreshIntervalSeconds"
    "discoveryConcurrency"
    "sensorsEnabled"
    "includeSensorThresholds"
    "metricsEnabled"
    "metricsFetchIntervalSeconds"
    "telemetryEnabled"
    "telemetryFetchIntervalSeconds"
    "firmwareEnabled"
    "firmwareRefreshIntervalSeconds"
    "managerEnabled"
    "managerPollIntervalSeconds"
    "leakDetectorEnabled"
    "leakDetectorPollIntervalSeconds"
    "leakDetectorStateRefreshIntervalSeconds"
    "logsEnabled"
    "logsMode"
    "logsSseInitialBackoffSeconds"
    "logsSseMaxBackoffSeconds"
    "logsCollectionIntervalSeconds"
    "logsStateRefreshIntervalSeconds"
    "logsExcludeServices"
    "logsSkipInitialHistory"
    "logsAutoSseNotAvailableThreshold"
    "logsAutoConnectFailureWindowSeconds"
    "logsAutoConnectFailureThreshold"
    "logsAutoRetrySseAfterDowngrade"
    "reachabilityEnabled"
    "reachabilityIntervalSeconds"
    "reachabilityTimeoutSeconds"
    "reachabilityLogMode"
    "rateLimitEnabled"
    "rateLimitBucketBurst"
    "rateLimitBucketReplenishMilliseconds"
    "rateLimitMaxJitterMilliseconds"
    -}}
{{- $value := get $config $key -}}
{{- if and (hasKey $config $key) (ne $value nil) -}}{{- $_ := set $nico $key $value -}}{{- end -}}
{{- end }}
{{ if $nico }}
[nico_health]
{{ range $key := keys $nico | sortAlpha -}}
{{ regexReplaceAll "([A-Z])" $key "_${1}" | lower }} = {{ get $nico $key | toJson }}
{{ end -}}
{{ end -}}
{{ range .Values.inventory.targets }}
[[targets]]
kind = {{ default "unknown" .kind | toJson }}
ip = {{ .address | toJson }}
mac = {{ .mac | toJson }}
{{ with .port }}port = {{ . }}
{{ end }}{{ with trim (.name | default "") }}name = {{ toJson . }}
{{ end }}{{ with trim (.computeZone | default "") }}compute_zone = {{ toJson . }}
{{ end }}{{ with trim (.nodeGroup | default "") }}node_group = {{ toJson . }}
{{ end }}{{ with trim (.credentialsProfile | default "") }}credentials_profile = {{ toJson . }}
{{ end }}{{ include "fleet-intelligence-oob-collector.tomlLabels" (default dict .labels) }}
{{ end }}
{{- end -}}
