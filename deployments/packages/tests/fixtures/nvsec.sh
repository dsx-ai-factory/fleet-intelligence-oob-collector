#!/usr/bin/env bash
# Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
set -euo pipefail
while (( $# )); do
    case "$1" in
        --input_file) source=$2; shift ;;
        --result_dir) result_dir=$2; shift ;;
        --result_filename) result_filename=$2; shift ;;
        --job_type) job_type=$2; shift ;;
    esac
    shift
done
printf '%s %s\n' "$job_type" "${source##*/}" >> "$SIGN_LOG"
[[ ${SIGN_MODE:-} != empty ]] || exit 0
target="$result_dir/$result_filename"
if [[ "$source" == *.deb ]]; then
    cp "$source" "$target"
    work=$(mktemp -d /tmp/signing-stub.XXXXXX)
    trap 'rm -rf "$work"' EXIT
    printf 'Version: 4\nSigner: builder\nFiles:\n' > "$work/manifest"
    while read -r member; do
        if [[ ${SIGN_MODE:-} == missing && "$member" == data.tar* ]]; then continue; fi
        ar p "$source" "$member" > "$work/payload"
        printf ' %s %s %s %s\n' "$(md5sum "$work/payload" | cut -d' ' -f1)" "$(sha1sum "$work/payload" | cut -d' ' -f1)" "$(stat -c %s "$work/payload")" "$member" >> "$work/manifest"
    done < <(ar t "$source")
    gpg --batch --yes --armor --clearsign --output "$work/_gpgbuilder" "$work/manifest"
    ar q "$target" "$work/_gpgbuilder"
else
    gpg --batch --yes --armor --detach-sign --output "$target" "$source"
fi
