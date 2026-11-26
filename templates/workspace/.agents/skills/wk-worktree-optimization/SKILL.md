---
name: wk-worktree-optimization
description: Review registered projects for repeated npm installs across worktrees and recommend pnpm migrations.
---

# Review dependency installs across worktrees

## Find projects that use npm for development

Run `wk status --json`. Review the projects the user named, or all registered projects for a workspace-wide review. Use the reported snapshot commit to inspect tracked files in `repos/<project>/` from the workspace session. For example, `git -C repos/web ls-tree -r --name-only <commit>` lists paths and `git -C repos/web show <commit>:README.md` reads the tracked README. If a snapshot is unavailable, mention the gap and continue. Keep this review read-only. It requires no installs, builds, disk measurements or task creation.

Identify each dependency root from its `package.json` and lockfile. In a monorepo, treat packages covered by one workspace lockfile as a group and check independently managed directories separately. A root qualifies when both of these signals are present.

- A tracked `package-lock.json` belongs to that root.
- Developer instructions, scripts or a CI build or test job run `npm ci` or `npm install` (including `npm i`) there to install project dependencies. Check the working directory of the install step.

An `npm publish` step, a global tool install or a manifest alone gives no evidence of a development install. Skip roots already using pnpm or another manager. If the lockfiles, `packageManager` field and install commands disagree, identify the conflicting paths and leave that root out of the migration recommendations.

## Report the match

For each candidate, name the project and dependency root, cite the lockfile and install entry point, and describe the edits that migration would require. Explain that each worktree keeps its own `node_modules` while pnpm can reuse package files from a shared store on the same filesystem. If nothing matches, say so briefly.

## Hand off an approved migration

When the user asks to apply a recommendation, create a wk task with only the selected projects, for example `wk start chore/web-pnpm web -D`. Confirm its members with `wk status <task>` and give the user a launch command for a fresh task session. Keep `repos/` read-only. Pass the evidence paths and the following steps to the task session for implementation in its member worktrees.

1. Confirm the install entry points at the task's starting commit. Choose a pnpm version compatible with the project's Node version and record it in `packageManager`. For an npm workspace, translate its package globs into `pnpm-workspace.yaml` first.
2. Run `pnpm import` at the dependency root while the npm lockfile is still present. Review the resolved dependencies before retiring the old lockfile.
3. Update the affected developer instructions, scripts, CI and container install steps, including pnpm provisioning and any lockfile-based cache keys. Preserve separate npm publishing steps where the project uses them.
4. Run `pnpm install --frozen-lockfile` and the project's existing build and tests in the task worktree.
