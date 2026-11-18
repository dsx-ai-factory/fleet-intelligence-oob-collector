{{/*
Copyright (c) 2025-2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
SPDX-License-Identifier: Apache-2.0
*/}}
{{/* Keep generated TOML limited to validated inventory and NICo values. */}}
{{- define "fleet-intelligence-oob-collector.tomlLabels" -}}
{{- $entries := list -}}
{{- range $name, $value := . -}}
{{- $entries = append $entries (printf "%s = %s" (toJson $name) (toJson $value)) -}}
{{- end -}}
{{- printf "labels = { %s }" (join ", " $entries) -}}
{{- end -}}

{{- define "fleet-intelligence-oob-collector.collectorConfig" -}}
{{ with .Values.inventory.computeZone }}compute_zone = {{ trim . | toJson }}
{{ end }}{{ with .Values.inventory.nodeGroup }}node_group = {{ trim . | toJson }}
{{ end }}{{ include "fleet-intelligence-oob-collector.tomlLabels" .Values.inventory.labels }}

{{ if .Values.nicoIntegration.enabled }}
[endpoint_source]
type = "nico"
api_url = {{ required "nicoIntegration.apiUrl is required" .Values.nicoIntegration.apiUrl | toJson }}
org = {{ required "nicoIntegration.org is required" .Values.nicoIntegration.org | toJson }}
site_id = {{ required "nicoIntegration.siteId is required" .Values.nicoIntegration.siteId | toJson }}
api_name = {{ .Values.nicoIntegration.apiName | toJson }}
token_file = "/run/secrets/nico-rest/access-token"
timeout_seconds = {{ .Values.nicoIntegration.timeoutSeconds }}
max_stale_seconds = {{ .Values.nicoIntegration.maxStaleSeconds }}

[nico_bmc_proxy]
url = {{ required "nicoIntegration.proxyUrl is required" .Values.nicoIntegration.proxyUrl | toJson }}
ca_file = "/run/secrets/nico-proxy/ca.crt"
client_cert_file = "/run/secrets/nico-proxy/tls.crt"
client_key_file = "/run/secrets/nico-proxy/tls.key"
{{ else }}
[credentials]
file = "/run/secrets/bmc/credentials.json"
{{ end }}
{{- $nico := dict -}}
{{- $config := dict -}}
{{- if not .Values.nicoIntegration.enabled -}}{{- $config = default dict .Values.nicoHardwareHealth.config -}}{{- end -}}
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
{{ if not .Values.nicoIntegration.enabled }}
{{ range .Values.inventory.targets }}
[[targets]]
kind = {{ default "unknown" .kind | toJson }}
ip = {{ .address | toJson }}
mac = {{ .mac | toJson }}
{{ with .port }}port = {{ . }}
{{ end }}{{ with .name }}name = {{ trim . | toJson }}
{{ end }}{{ with .computeZone }}compute_zone = {{ trim . | toJson }}
{{ end }}{{ with .nodeGroup }}node_group = {{ trim . | toJson }}
{{ end }}{{ with .credentialsProfile }}credentials_profile = {{ trim . | toJson }}
{{ end }}{{ include "fleet-intelligence-oob-collector.tomlLabels" (default dict .labels) }}
{{ end }}
{{ end }}
{{- end -}}
