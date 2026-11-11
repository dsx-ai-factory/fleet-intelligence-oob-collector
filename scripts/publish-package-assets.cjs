// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');

module.exports = async function uploadAssets({ github, context, releaseId, directory = 'dist' }) {
  const tag = context.ref.replace(/^refs\/tags\//, '');
  if (!/^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(tag)) throw new Error('Invalid release tag');
  const version = tag.slice(1);
  const expected = [
    `fleetint-oob_${version}_amd64.deb`,
    `fleetint-oob_${version}_arm64.deb`,
    `fleetint-oob_${version}_amd64.rpm`,
    `fleetint-oob_${version}_arm64.rpm`,
    'checksums.txt', 'checksums.txt.asc', 'fleet-intelligence.pub.asc',
    'verify-linux-package-signature.sh',
  ];
  // Validate the complete local inventory before beginning any upload.
  for (const name of expected) fs.accessSync(path.join(directory, name), fs.constants.R_OK);
  const { data: release } = await github.rest.repos.getRelease({ ...context.repo, release_id: releaseId });
  if (release.tag_name !== tag || !release.draft) throw new Error('Expected the reserved draft release');
  const assets = await github.paginate(github.rest.repos.listReleaseAssets, {
    ...context.repo, release_id: releaseId, per_page: 100,
  });
  if (assets.some(asset => !expected.includes(asset.name))) throw new Error('Unexpected draft assets');
  for (const name of expected) {
    const data = fs.readFileSync(path.join(directory, name));
    const digest = `sha256:${crypto.createHash('sha256').update(data).digest('hex')}`;
    const existing = assets.find(asset => asset.name === name);
    // Writers are serialized per tag by the release workflow. A failed upload
    // may leave an empty starter asset with no published bytes to preserve.
    if (existing && existing.state === 'starter' && existing.size === 0 && existing.digest == null) {
      await github.rest.repos.deleteReleaseAsset({ ...context.repo, asset_id: existing.id });
    } else if (existing) {
      if (existing.digest !== digest || existing.state !== 'uploaded') {
        throw new Error(`Refusing to replace existing release asset ${name}`);
      }
      continue;
    }
    const { data: uploaded } = await github.rest.repos.uploadReleaseAsset({
      ...context.repo, release_id: releaseId, name, data,
      headers: { 'content-type': 'application/octet-stream' },
    });
    if (uploaded.digest !== digest || uploaded.state !== 'uploaded') {
      throw new Error(`Uploaded asset failed digest verification: ${name}`);
    }
  }
};
