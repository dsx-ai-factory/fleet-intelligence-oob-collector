{{/*
Copyright (c) 2025-2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
SPDX-License-Identifier: Apache-2.0
*/}}
{{/* Keep generated TOML limited to validated inventory values. */}}
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

[credentials]
file = "/run/secrets/bmc/credentials.json"
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
