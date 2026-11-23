# workit-rs

English | [简体中文](README.zh-CN.md) · [Repository guide](AGENTS.md)

`wk` is a Rust CLI for working on a task across several Git repositories. It clones each project once under `repos/` and places a worktree for each participating project in a shared task directory. Start Codex, Claude Code, or a shell there to work across the projects.

Use `repos/<project>/` to browse the reference checkout. Start development with `wk start <task> <project...>` and edit, build, and commit in `tasks/<task>/<project>/`. Task commits share Git data with the reference clone while its checked-out files stay in place.

## Local development

Building from source requires Rust 1.94 or later and Git. The shortcuts below use `just`.

In this checkout, `just wk` and `cargo run --locked --` use the Git-ignored `.wk-dev/` workspace by default. Run `just wk init` once to create it. It holds the workspace configuration, clones, tasks, and worktrees. Cargo builds into `target/`, and automated tests use temporary directories.

```sh
just wk init
just wk status
just wk add ../chen/codex-ops
just wk start test-cli codex-ops
just wk run test-cli -- codex
```

For another workspace, run `just wk init /tmp/wk-demo`, then `just wk -w /tmp/wk-demo status`. Explicit paths take priority over the `WK_WORKSPACE` default supplied by the local Cargo configuration. The `add` command resolves relative repository paths from the invocation directory. When running from source, the launch commands printed by `start` can use the current build directly.

## Installation and usage

Once a version is published, either channel installs the `wk` command.

```sh
npm install -g workit-rs
# or
cargo install workit-rs --locked
```

```sh
wk init ~/work/cyder
cd ~/work/cyder
wk add git@github.com:cyder-hub/cyder-api.git
wk add git@github.com:ChenLuoi/codex-ops.git
wk start auth-refactor
```

In the project list, use ↑/↓ to move, Space to toggle each project, and Enter to confirm. Select both projects in this example. Press Enter again to use each project's configured `base`, or answer `n` to choose an `origin` branch for each project by prefix. Once all choices are complete, `wk` creates `tasks/auth-refactor/{cyder-api,codex-ops}/` and checks out a new `wk/auth-refactor` branch in each worktree. Choose one of the printed launch commands.

```sh
wk run auth-refactor -- codex
wk run auth-refactor -- claude
wk run auth-refactor -- code .
```

All three commands open the same directory, `tasks/auth-refactor/`. VS Code shows both repositories and the shared files in one window. Codex and Claude Code see those same files. `wk path auth-refactor` prints the directory for any other editor. Run tests, commit, and push from each project's own worktree.

## Commands

| Command | Behavior |
| --- | --- |
| `wk init [dir]` | Initialize the workspace, add the required ignore rules, and preserve existing configuration and instructions. |
| `wk add <url> [directory] [--base <ref>]` (alias: `wk clone`) | Clone an SSH, HTTPS, or local repository into a new `repos/<directory>/` and register it. The directory name is also the project name. |
| `wk sync [project...]` | Clone missing projects and fetch from `origin` for existing ones. |
| `wk remove <project>` (alias: `wk rm`) | Unregister the project and delete its clean local clone. If the clone is already missing, remove the registration. |
| `wk start <task> [project...] [-D] [--no-fetch]` | Select projects interactively or list them as arguments. `--default` / `-D` skips the base question. `--no-fetch` uses cached refs, including for branch selection. |
| `wk run <task> -- <program> [args...]` | Start a program in the task directory. |
| `wk path <task> [project]` | Print the absolute path to the task or one of its worktrees for use with a shell or editor. |
| `wk status [task] [--json] [--fetch]` | Show projects, task phases, branches, and file changes. Remote comparisons use local Git data. Pass `--fetch` to refresh it. |
| `wk doctor` | Check Git, configuration, project origins, and worktree consistency. Returns a nonzero exit code when it finds a problem. |
| `wk close <task>` | Check all member worktrees, remove them through Git, then delete the entire task directory and its shared resources. Keep task branches and the archived task record. |

With automatic workspace discovery, `wk` searches parent directories for the nearest `wk.yaml`. From outside the workspace, use `wk --workspace <dir> status`.

For a new task in a script, supply at least one project, for example `wk start auth-refactor cyder-api codex-ops -D`. Non-interactive runs use each project's configured base. A branch chosen interactively applies only to that task. Esc cancels before creating the task record, branches, or worktrees.

With only a URL, `wk clone` (or `wk add`) prompts for the directory name, then shows the remote branches with the default branch highlighted. Press Enter twice to accept both defaults. To choose another branch, type a case-sensitive prefix of its short name: `rel` matches `release/2.x`. Use ↑/↓ to select and Enter to confirm. The manifest stores the tracking ref, such as `origin/release/2.x`. Esc cancels before cloning.

Scripts use defaults without prompting. Supplying either the directory or `--base` also skips both prompts. Use `--base <ref>` to specify a tag or commit, or when the remote has no advertised default branch.

While Git looks up remote branches, clones a project, or fetches from `origin`, a spinner on the terminal shows the current operation. It uses stderr and is omitted when stderr is redirected.

```sh
wk clone git@github.com:cyder-hub/cyder-api.git api-preview --base origin/develop
```

The destination is a single directory name under `repos/`. A registered name or an existing destination (including an empty directory or matching clone) causes an error. Use a different name to add the same URL again. Use `wk sync <name>` to restore a missing clone for a registered project. An explicit base is resolved to a commit by `wk start`.

If a reference checkout has staged, modified, untracked, or ignored files, `wk status` reports the problem and `wk doctor` fails. The same checks flag a switch away from the known default branch or a reference HEAD with local-only commits. Operations on that project's clone or tasks stop until you preserve your work and restore the checkout. These checks are enforced by `wk`; the ordinary clone remains accessible through Git and your editor.

To remove a project, close its tasks, then run `wk rm <project>`. Open or interrupted tasks and linked worktrees (including manually created ones) block removal. Populated submodule directories and Git data in `.git/modules` also block removal, even when the parent repository reports a clean checkout. Before deleting the clone, `wk` checks commits across local refs and reflogs against locally known `origin` branches. An unpushed commit on a retained task branch still blocks removal after the task closes. Push the work you want to keep. Closed task records and the remote repository are preserved.

## Configuration and recovery

Commit `wk.yaml` to share the project list. After cloning a workspace, run `wk init` and `wk sync` to create its local directories and clone its projects.

```yaml
version: 1
projects:
  cyder-api:
    name: cyder-api
    url: git@github.com:cyder-hub/cyder-api.git
    base: origin/main
  codex-ops:
    name: codex-ops
    url: git@github.com:ChenLuoi/codex-ops.git
    base: origin/develop
```

Use the CLI for routine changes to `wk.yaml`. Every entry stores its name, URL, and base, including defaults. If a manual correction is necessary, keep it to individual values and keep the project key and its `name` identical. Avoid restructuring the manifest or adding comments: the CLI serializes it on write and may discard comments and custom formatting.

Place task resources beside the worktrees. For example, after adding a brief and fixtures, one task might contain:

```text
tasks/auth-refactor/
├── AGENTS.md       # generated by wk start
├── brief.md
├── fixtures/
├── cyder-api/      # Git worktree
└── codex-ops/      # Git worktree
```

The entire `tasks/` tree is ignored by the workspace repository. **Closing a task also deletes its brief, fixtures, editor settings, and other task-local files.** Commit anything you need to keep in a participating project or copy it outside the task directory first. The local record at `.wk/tasks/<task>.json` stores project membership, base refs, starting commits, and phase. Workspace writes use a file lock and replace configuration and task records atomically.

Once the task record exists, rerun `wk start <task>` to resume using its saved projects and commits. If you list projects on a retry, they must match the saved set. If closing is interrupted, resolve the reported error and rerun `wk close`. Before closing, handle modified, untracked, and ignored files, including local files such as `.env`. Existing task branches and archived records reserve their task names. The prototype uses ordinary Git clones and UTF-8 paths.

## Verification and release

```sh
just check
just smoke-interactive
just smoke-npm
cargo package --locked --registry crates-io
```

Integration tests create real Git repositories in temporary directories. On Linux and macOS, `just smoke-interactive` uses Node.js 20+ and the system `script` utility to exercise the prompts for adding projects and starting tasks. `just smoke-npm` packs the host binary and the launcher separately, installs them offline into a temporary npm prefix, and checks the version and workspace initialization.

Both registries use the package name `workit-rs`. The crate contains the Rust source. The npm launcher selects a prebuilt executable from an optional `@workit-rs/<platform>` package. The release targets are Linux x64 and arm64 (glibc and musl), macOS x64 and arm64, and Windows x64.

1. Set `CARGO_REGISTRY_TOKEN` and `NPM_TOKEN` in the GitHub `release` environment. The npm account needs permission to publish the main package and the public platform packages under `@workit-rs`.
2. Keep the version in `Cargo.toml`, `Cargo.lock`, and `package.json` in sync, including all optional dependency versions. Run the Release workflow manually to check the metadata, build matrix, and packages.
3. Push a matching tag such as `v0.1.0` to publish to both registries after all platform builds succeed. npm publishes the platform packages first and the launcher last. If a publishing job fails, rerun the failed jobs in the same workflow run.
