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

{{/* Only schema-validated NICo fields enter this recursive TOML writer.
Use explicit nil checks: false, zero jitter, and empty exclusion arrays are overrides. */}}
{{- define "fleet-intelligence-oob-collector.nicoToml" -}}
[{{ .path }}]
{{ range $key := keys .values | sortAlpha -}}
{{- $value := get $.values $key -}}
{{- if and (ne $value nil) (not (kindIs "map" $value)) -}}
{{ regexReplaceAll "([A-Z])" $key "_${1}" | lower }} = {{ $value | toJson }}
{{ end -}}
{{- end -}}
{{- range $key := keys .values | sortAlpha -}}
{{- $value := get $.values $key -}}
{{- if kindIs "map" $value }}
{{ include "fleet-intelligence-oob-collector.nicoToml" (dict "path" (printf "%s.%s" $.path (regexReplaceAll "([A-Z])" $key "_${1}" | lower)) "values" $value) }}
{{ end -}}
{{- end -}}
{{- end -}}

{{- define "fleet-intelligence-oob-collector.collectorConfig" -}}
{{ with .Values.inventory.computeZone }}compute_zone = {{ trim . | toJson }}
{{ end }}{{ with .Values.inventory.nodeGroup }}node_group = {{ trim . | toJson }}
{{ end }}{{ include "fleet-intelligence-oob-collector.tomlLabels" .Values.inventory.labels }}

[credentials]
file = "/run/secrets/bmc/credentials.json"
{{- $nico := dict -}}
{{- range $key := list "sensorFetchIntervalSeconds" "bmcRequestConcurrency" "endpointDiscoveryIntervalSeconds" "discovery" "sensors" "metrics" "telemetry" "firmware" "manager" "leakDetector" "logs" "reachability" "rateLimit" -}}
{{- $value := get $.Values.nicoHardwareHealth $key -}}
{{- if and (hasKey $.Values.nicoHardwareHealth $key) (ne $value nil) -}}{{- $_ := set $nico $key $value -}}{{- end -}}
{{- end }}
{{ if $nico }}
{{ include "fleet-intelligence-oob-collector.nicoToml" (dict "path" "nico_health" "values" $nico) }}
{{ end -}}
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
{{- end -}}
