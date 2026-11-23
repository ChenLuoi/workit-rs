# workit-rs

English | [简体中文](README.zh-CN.md) · [Repository guide](AGENTS.md)

`wk` is a Rust CLI for working on a task across several Git repositories. It clones each project once under `repos/` and places a worktree for each participating project in a shared task directory. Start Codex, Claude Code, or a shell there to work across the projects.

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
wk start auth-refactor cyder-api codex-ops
```

The last command creates `worktrees/auth-refactor/{cyder-api,codex-ops}/`, checks out a `wk/auth-refactor` branch in each repository, and prints launch commands. Choose a program you have installed.

```sh
wk run auth-refactor -- codex
wk run auth-refactor -- claude
```

`run` starts the program in the task directory and passes through arguments, terminal I/O, and the exit code. You can return to the same task after the program exits. Run tests, commit, and push from each project's own directory.

## Commands

| Command | Behavior |
| --- | --- |
| `wk init [dir]` | Initialize the workspace, add the required ignore rules, and preserve existing configuration and instructions. |
| `wk add <url> [--name alias] [--base origin/develop]` | Clone and register a project. The name defaults to the repository name in the URL, and the base defaults to `origin/HEAD`. Accepts SSH, HTTPS, and paths to existing local repositories. |
| `wk sync [project...]` | Clone missing projects and fetch from `origin` for existing ones. |
| `wk start <task> <project...> [--no-fetch]` | Fetch and create a worktree for each project from its base commit. Use `--no-fetch` to work offline. |
| `wk run <task> -- <program> [args...]` | Start a program in the task directory. |
| `wk path <task> [project]` | Print the absolute path to the task or one of its worktrees for use with a shell or editor. |
| `wk status [task] [--json] [--fetch]` | Show projects, task phases, branches, and file changes. Remote comparisons use local Git data. Pass `--fetch` to refresh it. |
| `wk doctor` | Check Git, configuration, project origins, and worktree consistency. Returns a nonzero exit code when it finds a problem. |
| `wk close <task>` | Check all members before removing clean worktrees. Keep task branches, notes in the task directory, and the archived task record. |

With automatic workspace discovery, `wk` searches parent directories for the nearest `wk.yaml`. From outside the workspace, use `wk --workspace <dir> status`.

## Configuration and recovery

Commit `wk.yaml` to share the project list. After cloning a workspace, run `wk init` and `wk sync` to create its local directories and clone its projects.

```yaml
version: 1
projects:
  cyder-api:
    url: git@github.com:cyder-hub/cyder-api.git
  codex-ops:
    url: git@github.com:ChenLuoi/codex-ops.git
    base: origin/develop
```

`wk add` serializes the YAML when it writes the manifest, so handwritten comments and formatting may be lost.

Put a shared task brief in `tasks/<task>.md`. The `AGENTS.md` in each task directory lists its projects and points to the brief. The local record at `.wk/tasks/<task>.json` stores the projects, base refs, starting commits, and task phase. Workspace writes use a file lock. Configuration and task records are written to temporary files and replaced atomically.

If task creation is interrupted, rerun `wk start` with the same name and project set to create the remaining worktrees at the recorded commits. If closing is interrupted, resolve the reported error and rerun `wk close`. Before closing, handle modified, untracked, and ignored files, including local files such as `.env`. Existing task branches and archived records reserve their task names. The prototype uses ordinary Git clones and UTF-8 paths.

## Verification and release

```sh
just check
just smoke-npm
cargo package --locked --registry crates-io
```

Integration tests create real Git repositories in temporary directories. `just smoke-npm` packs the host binary and the launcher separately, installs them offline into a temporary npm prefix, and checks the version and workspace initialization.

Both registries use the package name `workit-rs`. The crate contains the Rust source. The npm launcher selects a prebuilt executable from an optional `@workit-rs/<platform>` package. The release targets are Linux x64 and arm64 (glibc and musl), macOS x64 and arm64, and Windows x64.

1. Set `CARGO_REGISTRY_TOKEN` and `NPM_TOKEN` in the GitHub `release` environment. The npm account needs permission to publish the main package and the public platform packages under `@workit-rs`.
2. Keep the version in `Cargo.toml`, `Cargo.lock`, and `package.json` in sync, including all optional dependency versions. Run the Release workflow manually to check the metadata, build matrix, and packages.
3. Push a matching tag such as `v0.1.0` to publish to both registries after all platform builds succeed. npm publishes the platform packages first and the launcher last. If a publishing job fails, rerun the failed jobs in the same workflow run.
