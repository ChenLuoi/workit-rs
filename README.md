# workit-rs

English | [简体中文](README.zh-CN.md)

This repository develops `wk`, a Rust CLI that coordinates tasks across Git repositories. A workspace session selects the projects and starts a task. The user opens a fresh agent or editor session in the task's directory, with one linked worktree per project.

## Install

Download `wk-<platform>-v<version>` from [GitHub Releases](https://github.com/ChenLuoi/workit-rs/releases/latest). Pick `linux-{x64,arm64}-{gnu,musl}`, `darwin-{x64,arm64}`, or `win32-x64-msvc` for your system. Windows downloads end in `.exe`. Workspace operations require Git on `PATH`.

For a fresh mise installation, run `mise use -g github:ChenLuoi/workit-rs --minimum-release-age 0h` to install `wk`. To upgrade an existing installation, request `github:ChenLuoi/workit-rs@latest` explicitly.

The following commands install v0.1.3 directly on Linux x64 with glibc.

```sh
(
  set -e
  tmp=$(mktemp -d)
  trap 'rm -rf -- "$tmp"' EXIT
  cd "$tmp"
  version=0.1.3
  asset="wk-linux-x64-gnu-v$version"
  base="https://github.com/ChenLuoi/workit-rs/releases/download/v$version"
  curl -fsSLO "$base/$asset"
  curl -fsSLO "$base/SHA256SUMS"
  awk -v file="$asset" '$2 == file' SHA256SUMS | sha256sum -c -
  mkdir -p "$HOME/.local/bin"
  install -m 755 "$asset" "$HOME/.local/bin/wk"
  "$HOME/.local/bin/wk" --version
)
```

Add `~/.local/bin` to `PATH` to run `wk`. On macOS, select the Darwin asset and use `shasum -a 256 -c -` for the checksum step. On Windows, compare `Get-FileHash <download> -Algorithm SHA256` with `SHA256SUMS`, rename the download to `wk.exe`, and place it in a directory on `PATH`.

You can also install with `cargo install workit-rs --locked` or `npm install -g workit-rs`.

## Join a shared workspace

Clone the team's workspace repository, enter its directory and run `wk setup`. The first prompt offers personal settings for this checkout and shared settings for the team. Use `wk setup --local` or `wk setup --shared` to open either directly.

- Personal setup saves projects and tools in the Git-ignored `.wk/local.yaml`. Follow all shared projects or select the ones you use. For example, `wk setup --local api web` prepares just those two. Run `wk sync` after pulling team changes. Each failed project is reported and makes the command exit nonzero. Successful projects remain available.
- Shared setup edits tools, registered workspace skills and project descriptions in `wk.yaml`. For example, `wk setup --shared --tools codex,claude` enables those launchers for the team. Personal setup can select a subset.

To review repeated npm installs across worktrees, select **wk-worktree-optimization** under **Workspace skills** in `wk setup --shared`. Ask the workspace session which registered projects could benefit from pnpm. It reads lockfiles and install commands from the reference snapshots and suggests a migration for each match. When you choose a project to migrate, the workspace session creates a task and hands the changes to a fresh task session.

## Close a damaged task and find its history

Run `wk close <task>` even if its directory has already been deleted. For a damaged root repository or missing worktree, wk describes the damage and asks for confirmation. In scripts, `wk close <task> --force` confirms recovery. Confirming permanently deletes the remaining task files. The history record lists the damage under `damage`. Healthy worktrees must still pass the usual clean and unlocked checks.

Look in `.wk/history/` for closed tasks. Filenames start with the local date and time to the second, for example `2026—11—26—18—42—07—feature%2Flogin.yaml`. The fields `started_at` and `closed_at` use the same date format. A second archive for the same task and second gets the suffix `—2`.

## Develop from source

Use Rust 1.94 or later, Git and optionally `just`. Both `cargo run --locked -- <args>` and `just wk <args>` run the local build. The checkout sets `WK_WORKSPACE` to the ignored `.wk-dev/` directory. Cargo builds into `target/`. Tests create their own temporary workspaces.

Try the lifecycle against a Git remote:

```sh
just wk init --yes --tools codex --skills wk-cli,wk-workspace-workflow
just wk add https://github.com/ChenLuoi/codex-ops.git --description "Codex operations tooling"
just wk start feature/operations codex-ops -D
just wk status feature/operations
```

An empty project list makes `wk start` point to `wk add <url>`. With no active tasks, `wk run` points to `wk start`.

`wk start` creates an empty bare repository at `tasks/<task>/.git` so Git and Codex discover the task root. Run Git commands in the member worktrees to review and commit code. Shared task guides, skills and notes are untracked and are removed by `wk close`.

To work on the task, start a new session using the launch command printed by `start`. Run `just wk help` to browse commands by workflow, or `just wk help start` for task creation options. Use `-w <dir>` to target another workspace. `just clean-dev` deletes the entire local development workspace, including its Git data.

## Where to make changes

| Area | Source |
| --- | --- |
| CLI and prompts | `src/main.rs`, `src/prompt.rs`, `src/tool.rs` |
| Workspace configuration, reference snapshots and Git | `src/workspace.rs`, `src/git.rs` |
| Task lifecycle, status and diagnostics | `src/tasks.rs`, `src/report.rs`, `src/report/` |
| Generated workspace and task documents | `templates/workspace/`, `templates/task/`, `src/templates.rs` |
| Official workspace skills | `templates/workspace/.agents/skills/`, `src/templates/skills.rs` |

The workspace guides are starting points for teams to edit. The bundled **wk-cli** skill documents the command interface. **wk-workspace-workflow** covers project selection, handoff and closure. Edit their source files alongside the corresponding CLI behavior. Cargo embeds them in the binary for both release channels. After upgrading the binary, a workspace can run `wk doctor` to check registered skills. Use `wk sync --skills wk-cli` to replace that skill directory with the bundled copy, or `wk sync --skills` for all registered skills. Run project sync separately.

Task guides receive a snapshot of each member project's name, role, and actual starting ref and commit. Use the same member data for the generated README and AGENTS file. The integration tests in `tests/cli.rs` exercise these contracts through the built binary.

Task skills live in the workspace's versioned `task-skills/<id>/` catalog. The preset commands in `src/preset.rs` manage named combinations of projects, optional base overrides and skills in `wk.yaml`. Try `just wk preset add api-web` followed by `just wk start feature/login --preset api-web`. Add `--interactive` for one-off changes, or specify projects and `--skills one,two` directly. `src/task_skills.rs` snapshots selected directories for the fresh task session and interrupted creation.

## Verification and release

```sh
just check
just smoke-interactive
just smoke-npm
cargo package --locked --registry crates-io
```

CI runs on Ubuntu, macOS and Windows when a pull request opens, reopens or receives new commits. Every push to `master`, including a PR merge, runs CI on that branch for the pushed commit. Version tags run the same checks through the Release workflow. Each platform's integration tests create real Git repositories in temporary directories and build the release binary used by the smoke tests.

On Linux and macOS, `just smoke-interactive` uses Node.js 20+ and `socat` to exercise the prompts for initializing workspaces, adding projects, starting tasks and configuring and launching tools. `just smoke-npm` packs the host binary and the launcher separately, installs them offline into a temporary npm prefix, and checks the version and workspace initialization.

Both registries use the package name `workit-rs`. The crate contains the Rust source. The npm launcher selects a prebuilt executable from an optional `@workit-rs/<platform>` package. The release targets are Linux x64 and arm64 (glibc and musl), macOS x64 and arm64, and Windows x64.

1. Set `CARGO_REGISTRY_TOKEN` and `NPM_TOKEN` in the GitHub `release` environment. For the first release, verify the crates.io account email and grant the token permission to create `workit-rs`. The npm publishing account needs access to the unscoped launcher and all seven public packages under `@workit-rs`. Token-based CI publishing uses a granular write token with 2FA bypass.
2. Keep the version in `Cargo.toml`, `Cargo.lock`, and `package.json` in sync, including all optional dependency versions. Run the Release workflow manually to check the metadata, build matrix, and packages.
3. Push a matching tag such as `v0.1.0` on the tested commit. Publication waits for three-system CI and installation of the seven packaged targets. The musl binaries run in Alpine. npm publishes the platform packages first and the launcher last. If a publishing job fails, rerun the failed jobs in the same workflow run to reuse the original artifacts.

The rehearsal also runs the npm publisher in dry-run mode on all eight archives. If fixing the publisher itself requires a later commit, manually dispatch Release from the default branch and set `npm_run_id` to the original tag run ID. Recovery verifies the tag commit and its successful builds, then publishes the original npm archives. The manifest version on the default branch must match the tag.
