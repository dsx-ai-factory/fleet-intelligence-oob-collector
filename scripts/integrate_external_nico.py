#!/usr/bin/env python3
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0

"""Apply the FI-generated integration contract to a same-namespace NICo Deployment."""

import argparse
import copy
import json
import os
import posixpath
import re
import subprocess
import sys


DNS_LABEL = re.compile(r"^[a-z0-9]([-a-z0-9]*[a-z0-9])?$")
INTEGRATION_LABEL = "fleet-intelligence.nvidia.com/otel-source"
CHECKSUM_ANNOTATION = "fleet-intelligence.nvidia.com/config-checksum"
REVISION_ANNOTATION = "fleet-intelligence.nvidia.com/qualified-nico-revision"
SOURCE_IMAGE_ANNOTATION = "fleet-intelligence.nvidia.com/source-nico-image"
BASE_CONFIG_PATH_ANNOTATION = "fleet-intelligence.nvidia.com/base-config-path"
BASE_CONFIG_PLACEHOLDER = "__FLEET_INTELLIGENCE_EXISTING_NICO_CONFIG__"
BASE_CONFIG_MOUNT_PATH = "/etc/fleet-intelligence/external-nico-base"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Configure and restart an existing NICo Deployment from an installed "
            "Fleet Intelligence Helm release."
        )
    )
    parser.add_argument("--namespace", required=True)
    parser.add_argument("--release", required=True)
    parser.add_argument("--deployment", required=True)
    parser.add_argument("--container", required=True)
    parser.add_argument(
        "--existing-config-path",
        required=True,
        help=(
            "absolute path used by the existing NICo container for its TOML config"
        ),
    )
    parser.add_argument(
        "--confirm-qualified-revision",
        required=True,
        help="NICo source revision used by the FI-qualified target image",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="validate with the API server without changing the Deployment",
    )
    parser.add_argument(
        "--no-wait",
        action="store_true",
        help="return after patching instead of waiting for the rollout",
    )
    return parser.parse_args()


def fail(message: str) -> "NoReturn":
    raise SystemExit(message)


def validate_name(flag: str, value: str) -> None:
    if len(value) > 253 or not DNS_LABEL.fullmatch(value):
        fail(f"{flag} must be a lowercase Kubernetes DNS label")


def kubectl(*arguments: str, input_text: str | None = None) -> str:
    command = [os.environ.get("KUBECTL", "kubectl"), *arguments]
    result = subprocess.run(
        command,
        input=input_text,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        detail = result.stderr.strip() or result.stdout.strip()
        fail(f"{' '.join(command)} failed: {detail}")
    return result.stdout


def get_json(namespace: str, resource: str, *arguments: str) -> dict:
    output = kubectl("-n", namespace, "get", resource, *arguments, "-o", "json")
    try:
        return json.loads(output)
    except json.JSONDecodeError as error:
        fail(f"kubectl returned invalid JSON for {resource}: {error}")


def merge_named(existing: list[dict], desired: list[dict]) -> list[dict]:
    desired_names = {item["name"] for item in desired}
    return [item for item in existing if item.get("name") not in desired_names] + desired


def validate_image_transition(
    current_image: str, expected_source_image: str, qualified_image: str
) -> None:
    if current_image not in {expected_source_image, qualified_image}:
        fail(
            f"NICo container image {current_image!r} matches neither the expected "
            f"source image {expected_source_image!r} nor the FI-qualified image "
            f"{qualified_image!r}"
        )


def find_integration(namespace: str, release: str) -> dict:
    selector = (
        f"app.kubernetes.io/instance={release},"
        "app.kubernetes.io/component=external-nico-integration"
    )
    config_maps = get_json(namespace, "configmap", "-l", selector).get("items", [])
    if len(config_maps) != 1:
        fail(
            f"expected one external NICo integration ConfigMap for release {release!r}, "
            f"found {len(config_maps)}"
        )
    encoded = config_maps[0].get("data", {}).get("integration.json")
    if not encoded:
        fail("external NICo integration ConfigMap has no integration.json")
    try:
        return json.loads(encoded)
    except json.JSONDecodeError as error:
        fail(f"external NICo integration contract is invalid: {error}")


def path_in_mount(path: str, mount_path: str) -> bool:
    path = posixpath.normpath(path)
    mount_path = posixpath.normpath(mount_path)
    return path == mount_path or path.startswith(f"{mount_path.rstrip('/')}/")


def mount_paths_overlap(first: str, second: str) -> bool:
    return path_in_mount(first, second) or path_in_mount(second, first)


def replace_config_reference(
    container: dict,
    existing_config_path: str,
    generated_config_path: str,
    already_integrated: bool,
) -> None:
    source_path = generated_config_path if already_integrated else existing_config_path
    locations = []
    for field in ("command", "args"):
        for index, value in enumerate(container.get(field, [])):
            if value == source_path:
                locations.append((field, index))
    if len(locations) != 1:
        state = "generated" if already_integrated else "existing"
        fail(
            f"expected the {state} NICo config path {source_path!r} exactly once "
            "in the selected container command or arguments"
        )
    field, index = locations[0]
    container[field][index] = generated_config_path


def reject_reserved_name_collisions(
    existing: list[dict], desired: list[dict], resource: str, already_integrated: bool
) -> None:
    existing_by_name = {
        item.get("name"): item for item in existing if item.get("name") is not None
    }
    for item in desired:
        name = item["name"]
        current = existing_by_name.get(name)
        if current is not None and current != item and not already_integrated:
            fail(
                f"existing NICo pod has a {resource} named {name!r} reserved by "
                "the FI integration"
            )


def existing_integration_config_source(
    pod_spec: dict, init_container_name: str
) -> tuple[dict, str] | None:
    matches = [
        container
        for container in pod_spec.get("initContainers", [])
        if container.get("name") == init_container_name
    ]
    if len(matches) > 1:
        fail(f"found multiple existing {init_container_name!r} init containers")
    if not matches:
        return None
    container = matches[0]
    arguments = container.get("args", [])
    try:
        base_path = arguments[arguments.index("--base-config") + 1]
    except (ValueError, IndexError):
        return None
    mounts = [
        mount
        for mount in container.get("volumeMounts", [])
        if path_in_mount(base_path, mount.get("mountPath", ""))
    ]
    if not mounts:
        fail(
            "existing FI NICo integration does not have a base-config mount"
        )
    longest = max(len(mount["mountPath"]) for mount in mounts)
    most_specific = [
        mount for mount in mounts if len(mount["mountPath"]) == longest
    ]
    if len(most_specific) != 1:
        fail("existing FI NICo integration has ambiguous base-config mounts")
    return copy.deepcopy(most_specific[0]), base_path


def config_source_from_main_container(
    container: dict, existing_config_path: str
) -> tuple[dict, str]:
    normalized_path = posixpath.normpath(existing_config_path)
    if not normalized_path.startswith("/"):
        fail("--existing-config-path must be absolute")
    candidates = []
    for mount in container.get("volumeMounts", []):
        mount_path = mount.get("mountPath", "")
        if path_in_mount(normalized_path, mount_path):
            candidates.append(mount)
    if not candidates:
        fail(
            f"existing NICo config {existing_config_path!r} is not covered by a "
            "volume mount on the selected container"
        )
    source = max(candidates, key=lambda mount: len(mount["mountPath"]))
    source_path = posixpath.normpath(source["mountPath"])
    if source.get("subPath") or source.get("subPathExpr"):
        if normalized_path != source_path:
            fail(
                "a subPath NICo config mount must match --existing-config-path exactly"
            )
        mounted = copy.deepcopy(source)
        mounted["mountPath"] = f"{BASE_CONFIG_MOUNT_PATH}/config.toml"
        mounted["readOnly"] = True
        return mounted, mounted["mountPath"]

    relative_path = posixpath.relpath(normalized_path, source_path)
    mounted = {
        "name": source["name"],
        "mountPath": BASE_CONFIG_MOUNT_PATH,
        "readOnly": True,
    }
    return mounted, posixpath.join(BASE_CONFIG_MOUNT_PATH, relative_path)


def configure_preserved_base(
    pod_spec: dict,
    container: dict,
    desired_init: dict,
    existing_config_path: str | None,
    previous_config_path: str | None,
) -> tuple[dict, str]:
    if not existing_config_path:
        fail("--existing-config-path is required")
    normalized_path = posixpath.normpath(existing_config_path)
    if previous_config_path and normalized_path != previous_config_path:
        fail(
            "--existing-config-path does not match the path recorded by the existing "
            "FI integration"
        )

    existing_source = existing_integration_config_source(
        pod_spec, desired_init["name"]
    )
    if existing_source:
        source_mount, base_path = existing_source
    else:
        source_mount, base_path = config_source_from_main_container(
            container, normalized_path
        )

    volume_names = {volume.get("name") for volume in pod_spec.get("volumes", [])}
    if source_mount.get("name") not in volume_names:
        fail(
            f"existing NICo config volume {source_mount.get('name')!r} is not "
            "defined in the pod"
        )

    desired_init = copy.deepcopy(desired_init)
    arguments = desired_init.get("args", [])
    placeholder_count = arguments.count(BASE_CONFIG_PLACEHOLDER)
    if placeholder_count != 1:
        fail(
            "external NICo integration contract must contain exactly one base-config placeholder"
        )
    desired_init["args"] = [
        base_path if argument == BASE_CONFIG_PLACEHOLDER else argument
        for argument in arguments
    ]
    desired_init["volumeMounts"] = merge_named(
        desired_init.get("volumeMounts", []), [source_mount]
    )
    return desired_init, normalized_path


def mutate_template(
    template: dict,
    container_name: str,
    integration: dict,
    existing_config_path: str | None = None,
) -> dict:
    metadata = template.setdefault("metadata", {})
    metadata.setdefault("labels", {})[INTEGRATION_LABEL] = integration["sourceLabel"]
    annotations = metadata.setdefault("annotations", {})
    was_integrated = CHECKSUM_ANNOTATION in annotations
    annotations[CHECKSUM_ANNOTATION] = integration["configChecksum"]
    annotations[REVISION_ANNOTATION] = integration["qualifiedNicoRevision"]
    annotations[SOURCE_IMAGE_ANNOTATION] = integration["expectedSourceNicoImage"]

    pod_spec = template.setdefault("spec", {})
    containers = pod_spec.get("containers", [])
    matches = [container for container in containers if container.get("name") == container_name]
    if len(matches) != 1:
        fail(
            f"expected one container named {container_name!r} in the NICo Deployment, "
            f"found {len(matches)}"
        )
    container = matches[0]
    desired_main = copy.deepcopy(integration["mainContainer"])
    already_integrated = BASE_CONFIG_PATH_ANNOTATION in annotations
    desired_init = integration["initContainer"]
    desired_init, recorded_path = configure_preserved_base(
        pod_spec,
        container,
        desired_init,
        existing_config_path,
        annotations.get(BASE_CONFIG_PATH_ANNOTATION),
    )
    annotations[BASE_CONFIG_PATH_ANNOTATION] = recorded_path

    container["image"] = integration["qualifiedNicoImage"]
    generated_config_path = desired_main["args"][0]
    replace_config_reference(
        container,
        posixpath.normpath(existing_config_path),
        generated_config_path,
        already_integrated,
    )
    existing_mounts = container.get("volumeMounts", [])
    desired_main["volumeMounts"] = [
        mount
        for mount in desired_main["volumeMounts"]
        if path_in_mount(generated_config_path, mount["mountPath"])
        or not any(
            existing.get("mountPath") == mount["mountPath"]
            and existing.get("name") != mount["name"]
            for existing in existing_mounts
        )
    ]
    container["env"] = merge_named(container.get("env", []), desired_main.get("env", []))

    desired_mounts = desired_main["volumeMounts"]
    desired_names = {mount["name"] for mount in desired_mounts}
    desired_paths = {mount["mountPath"] for mount in desired_mounts}
    container["volumeMounts"] = [
        mount
        for mount in container.get("volumeMounts", [])
        if mount.get("name") not in desired_names
        and not any(
            mount_paths_overlap(mount.get("mountPath", ""), desired_path)
            for desired_path in desired_paths
        )
    ] + desired_mounts

    retained_init_containers = []
    for init_container in pod_spec.get("initContainers", []):
        if init_container.get("name") == desired_init["name"]:
            continue
        retained_init_containers.append(init_container)
    reject_reserved_name_collisions(
        pod_spec.get("initContainers", []),
        [desired_init],
        "init container",
        was_integrated,
    )
    pod_spec["initContainers"] = retained_init_containers + [desired_init]
    used_integration_volumes = {
        mount["name"]
        for workload in [desired_init, desired_main]
        for mount in workload.get("volumeMounts", [])
    }
    desired_volumes = [
        volume
        for volume in integration["volumes"]
        if volume["name"] in used_integration_volumes
    ]
    reject_reserved_name_collisions(
        pod_spec.get("volumes", []),
        desired_volumes,
        "volume",
        was_integrated,
    )
    pod_spec["volumes"] = merge_named(
        pod_spec.get("volumes", []), desired_volumes
    )
    pod_spec["imagePullSecrets"] = merge_named(
        pod_spec.get("imagePullSecrets", []), integration.get("imagePullSecrets", [])
    )
    return template


def main() -> None:
    args = parse_args()
    for flag, value in [
        ("--namespace", args.namespace),
        ("--release", args.release),
        ("--deployment", args.deployment),
        ("--container", args.container),
    ]:
        validate_name(flag, value)

    integration = find_integration(args.namespace, args.release)
    required_revision = integration.get("qualifiedNicoRevision")
    if args.confirm_qualified_revision != required_revision:
        fail(
            "the confirmed NICo revision does not match the FI-qualified revision "
            f"{required_revision}"
        )

    deployment = get_json(args.namespace, f"deployment/{args.deployment}")
    containers = deployment.get("spec", {}).get("template", {}).get("spec", {}).get(
        "containers", []
    )
    image = next(
        (
            container.get("image", "<unknown>")
            for container in containers
            if container.get("name") == args.container
        ),
        "<missing>",
    )
    qualified_image = integration.get("qualifiedNicoImage")
    expected_source_image = integration.get("expectedSourceNicoImage")
    validate_image_transition(image, expected_source_image, qualified_image)
    template = mutate_template(
        deployment["spec"]["template"],
        args.container,
        integration,
        args.existing_config_path,
    )
    patch = {
        "metadata": {"resourceVersion": deployment["metadata"]["resourceVersion"]},
        "spec": {"template": template},
    }
    patch_arguments = [
        "-n",
        args.namespace,
        "patch",
        f"deployment/{args.deployment}",
        "--type=merge",
        "--patch",
        json.dumps(patch, separators=(",", ":")),
    ]
    if args.dry_run:
        patch_arguments.extend(["--dry-run=server", "-o", "name"])
    kubectl(*patch_arguments)

    action = "Validated" if args.dry_run else "Integrated"
    print(
        f"{action} NICo Deployment {args.namespace}/{args.deployment} "
        f"(container {args.container}, image {image} -> {qualified_image}, "
        f"qualified revision {required_revision})"
    )
    if not args.dry_run and not args.no_wait:
        kubectl(
            "-n",
            args.namespace,
            "rollout",
            "status",
            f"deployment/{args.deployment}",
            "--timeout=10m",
        )
        print(f"NICo rollout {args.namespace}/{args.deployment} is ready")


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(130)
