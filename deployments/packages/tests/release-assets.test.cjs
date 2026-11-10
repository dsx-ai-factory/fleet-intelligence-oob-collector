// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const crypto = require('node:crypto');
const upload = require('../../../scripts/publish-package-assets.cjs');

function fixture() {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'release-assets-'));
  const names = ['fleetint-oob_1.2.3_amd64.deb',
    'fleetint-oob_1.2.3_arm64.deb',
    'fleetint-oob_1.2.3_amd64.rpm', 'fleetint-oob_1.2.3_arm64.rpm', 'checksums.txt',
    'checksums.txt.asc', 'fleet-intelligence.pub.asc', 'verify-linux-package-signature.sh'];
  for (const name of names) fs.writeFileSync(path.join(directory, name), name);
  const state = { draft: true, tag_name: 'v1.2.3', assets: [], uploads: 0, fail: false };
  const github = {
    paginate: async () => state.assets,
    rest: { repos: {
      getRelease: async () => ({ data: state }),
      listReleaseAssets() {},
      deleteReleaseAsset: async ({ asset_id }) => {
        if (state.deleteFails) throw new Error('delete interrupted');
        const index = state.assets.findIndex(asset => asset.id === asset_id);
        assert.notEqual(index, -1);
        state.assets.splice(index, 1);
      },
      uploadReleaseAsset: async ({ name, data, headers }) => {
        assert.equal(headers?.['content-type'], 'application/octet-stream');
        assert.ok(Buffer.isBuffer(data));
        assert.deepEqual(data, fs.readFileSync(path.join(directory, name)));
        if (state.fail && state.uploads === 1) throw new Error('interrupted');
        state.uploads++;
        const asset = { name, state: 'uploaded', digest: 'sha256:' + crypto.createHash('sha256').update(data).digest('hex') };
        state.assets.push(asset);
        return { data: asset };
      },
    } },
  };
  return { directory, state, args: { github, context: { repo: { owner: 'example', repo: 'oob' }, ref: 'refs/tags/v1.2.3' }, releaseId: 1, directory },
    cleanup: () => fs.rmSync(directory, { recursive: true, force: true }) };
}

test('uploads binary assets with their media type and retries without replacing bytes', async () => {
  const f = fixture();
  try {
    fs.writeFileSync(path.join(f.directory, 'fleetint-oob_1.2.3_amd64.deb'), Buffer.from([0, 255, 128, 13, 10]));
    await upload(f.args);
    assert.equal(f.state.uploads, 8);
    await upload(f.args);
    assert.equal(f.state.uploads, 8);
  } finally { f.cleanup(); }
});

test('resumes an interrupted draft upload', async () => {
  const f = fixture();
  try {
    f.state.fail = true;
    await assert.rejects(upload(f.args));
    assert.equal(f.state.uploads, 1);
    f.state.fail = false;
    await upload(f.args);
    assert.equal(f.state.uploads, 8);
  } finally { f.cleanup(); }
});

for (const problem of ['published', 'tag', 'foreign', 'changed']) {
  test(`rejects ${problem} release state`, async () => {
    const f = fixture();
    try {
      if (problem === 'published') f.state.draft = false;
      if (problem === 'tag') f.state.tag_name = 'v9.9.9';
      if (problem === 'foreign') f.state.assets.push({ name: 'unsigned.exe' });
      if (problem === 'changed') f.state.assets.push({ name: 'fleetint-oob_1.2.3_amd64.deb', digest: 'sha256:bad', state: 'uploaded' });
      await assert.rejects(upload(f.args));
      assert.equal(f.state.uploads, 0);
    } finally { f.cleanup(); }
  });
}

test('recovers an abandoned empty starter asset after an upload failure', async () => {
  const f = fixture();
  try {
    const name = 'fleetint-oob_1.2.3_amd64.deb';
    f.state.assets.push({ id: 42, name, state: 'starter', size: 0, digest: null });
    await upload(f.args);
    assert.equal(f.state.uploads, 8);
    assert.equal(f.state.assets.length, 8);
    assert.ok(f.state.assets.every(asset => asset.state === 'uploaded'));
    await upload(f.args);
    assert.equal(f.state.uploads, 8);
  } finally { f.cleanup(); }
});
for (const problem of ['nonempty', 'digest', 'delete']) {
  test(`preserves starter assets when recovery is unsafe or fails: ${problem}`, async () => {
    const f = fixture();
    try {
      const asset = { id: 42, name: 'fleetint-oob_1.2.3_amd64.deb', state: 'starter', size: 0, digest: null };
      if (problem === 'nonempty') asset.size = 1;
      if (problem === 'digest') asset.digest = 'sha256:unexpected';
      if (problem === 'delete') f.state.deleteFails = true;
      f.state.assets.push(asset);
      await assert.rejects(upload(f.args));
      assert.deepEqual(f.state.assets, [asset]);
      assert.equal(f.state.uploads, 0);
    } finally { f.cleanup(); }
  });
}


test('uploads RPMs alongside DEBs and retains immutable retries', async () => {
  const f = fixture();
  try {
    await upload(f.args);
    assert.equal(f.state.uploads, 8);
    await upload(f.args);
    assert.equal(f.state.uploads, 8);
    fs.appendFileSync(path.join(f.directory, 'fleetint-oob_1.2.3_amd64.rpm'), 'changed');
    await assert.rejects(upload(f.args));
    assert.equal(f.state.uploads, 8);
  } finally { f.cleanup(); }
});

test('rejects an incomplete RPM set before uploading any assets', async () => {
  const f = fixture();
  try {
    fs.unlinkSync(path.join(f.directory, 'fleetint-oob_1.2.3_arm64.rpm'));
    await assert.rejects(upload(f.args));
    assert.equal(f.state.uploads, 0);
  } finally { f.cleanup(); }
});
const reserveRelease = require('../../../scripts/reserve-package-release.cjs');

function reservationFixture(releases = [], ref = 'refs/tags/v1.2.3') {
  const created = [];
  const args = {
    context: { repo: { owner: 'example', repo: 'oob' }, ref, sha: 'release-commit' },
    github: {
      paginate: async () => releases,
      rest: { repos: {
        listReleases() {},
        createRelease: async params => { created.push(params); return { data: { id: 42 } }; },
      } },
    },
  };
  return { args, created };
}

for (const tag of ['v1.2.3', 'v1.2.3-rc.1']) {
  test(`reserves a new draft for ${tag}`, async () => {
    const f = reservationFixture([], `refs/tags/${tag}`);
    assert.equal(await reserveRelease(f.args), 42);
    assert.equal(f.created.length, 1);
    assert.equal(f.created[0].tag_name, tag);
    assert.equal(f.created[0].target_commitish, 'release-commit');
    assert.equal(f.created[0].draft, true);
    assert.equal(f.created[0].prerelease, tag.includes('-'));
  });
}

test('reuses the matching draft after a full workflow rerun', async () => {
  const f = reservationFixture([{ id: 7, tag_name: 'v1.2.3', draft: true,
    target_commitish: 'release-commit' }]);
  assert.equal(await reserveRelease(f.args), 7);
  assert.equal(f.created.length, 0);
});

for (const problem of ['published', 'commit', 'ambiguous']) {
  test(`refuses ${problem} release reservation`, async () => {
    const release = { id: 7, tag_name: 'v1.2.3', draft: true, target_commitish: 'release-commit' };
    if (problem === 'published') release.draft = false;
    if (problem === 'commit') release.target_commitish = 'unrelated-commit';
    const f = reservationFixture(problem === 'ambiguous' ? [release, { ...release, id: 8 }] : [release]);
    await assert.rejects(reserveRelease(f.args), /Refusing to reuse/);
    assert.equal(f.created.length, 0);
  });
}

test('requires both RPMs when publishing a new release', async () => {
  const f = fixture();
  try {
    for (const arch of ['amd64', 'arm64']) {
      fs.unlinkSync(path.join(f.directory, `fleetint-oob_1.2.3_${arch}.rpm`));
    }
    await assert.rejects(upload(f.args));
    assert.equal(f.state.uploads, 0);
  } finally { f.cleanup(); }
});
