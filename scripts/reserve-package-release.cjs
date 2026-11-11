// Copyright (c) 2026, NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

module.exports = async function reserveRelease({ github, context }) {
  const tag = context.ref.replace(/^refs\/tags\//, '');
  const releases = await github.paginate(github.rest.repos.listReleases, {
    ...context.repo, per_page: 100,
  });
  const matches = releases.filter(release => release.tag_name === tag);
  if (matches.length) {
    const release = matches[0];
    if (matches.length !== 1 || !release.draft || release.target_commitish !== context.sha) {
      throw new Error(`Refusing to reuse release ${tag}: expected one draft targeting ${context.sha}`);
    }
    return release.id;
  }
  const { data } = await github.rest.repos.createRelease({
    ...context.repo, tag_name: tag, target_commitish: context.sha,
    name: tag, draft: true, prerelease: tag.includes('-'),
    generate_release_notes: true,
  });
  return data.id;
};
