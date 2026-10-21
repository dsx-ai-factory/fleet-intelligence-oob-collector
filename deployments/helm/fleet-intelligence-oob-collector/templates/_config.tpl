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

{{/* Derive the same telemetry labels the collector adds to its target inventory. */}}
{{- define "fleet-intelligence-oob-collector.targetLabels" -}}
{{- $root := .root -}}
{{- $target := .target -}}
{{- $labels := mergeOverwrite (deepCopy $root.Values.inventory.labels) (default dict $target.labels) -}}
{{- $reserved := list "bmc_endpoint" "bmc_ip" "collector_type" "component_type" "driver_version" "endpoint_ip" "endpoint_key" "endpoint_mac" "gpu_chassis_serial" "gpu_model" "gpu_serial" "gpu_uuid" "machine_id" "machine_serial" "machine_slot_number" "machine_tray_index" "machine_name" "compute_zone" "node_group" "nvlink_domain_uuid" "oob" "power_shelf_id" "power_shelf_serial_number" "rack_id" "serial_number" "switch_endpoint" "switch_endpoint_role" "switch_id" "switch_ip" "switch_is_primary" "switch_serial_number" "switch_slot_number" "switch_tray_index" "system_uuid" -}}
{{- range $key, $value := $labels -}}
{{- if has $key $reserved -}}{{- fail (printf "inventory labels key %q is reserved by NICo" $key) -}}{{- end -}}
{{- end -}}
{{- $_ := set $labels "oob" "true" -}}
{{- with $target.name -}}{{- $_ := set $labels "machine_name" (trim .) -}}{{- end -}}
{{- with (default $root.Values.inventory.computeZone $target.computeZone) -}}{{- $_ := set $labels "compute_zone" (trim .) -}}{{- end -}}
{{- with (default $root.Values.inventory.nodeGroup $target.nodeGroup) -}}{{- $_ := set $labels "node_group" (trim .) -}}{{- end -}}
{{- range $key, $value := $labels -}}
{{- if gt (len $value) 1024 -}}{{- fail (printf "inventory label %q exceeds 1024 bytes" $key) -}}{{- end -}}
{{- end -}}
{{- if gt (len $labels) 32 -}}{{- fail "inventory target exceeds the NICo limit of 32 labels including derived labels" -}}{{- end -}}
{{- toJson $labels -}}
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
{{ end }}{{ with .credentialsProfile }}credentials_profile = {{ . | toJson }}
{{ end }}{{ include "fleet-intelligence-oob-collector.tomlLabels" (default dict .labels) }}
{{ end }}
{{- end -}}
