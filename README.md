# workit-rs

English | [简体中文](README.zh-CN.md)

This repository develops `wk`, a Rust CLI that coordinates tasks across Git repositories. A workspace session selects the projects and starts a task. The user opens a fresh agent or editor session in the task's directory, with one linked worktree per project.

Install the CLI with `cargo install workit-rs --locked` or `npm install -g workit-rs`. Both install the `wk` command and require Git for workspace operations.

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

To work on the task, start a new session using the launch command printed by `start`. Run `just wk --help` for the command inventory. Use `-w <dir>` to target another workspace. `just clean-dev` deletes the entire local development workspace, including its Git data.

## Where to make changes

| Area | Source |
| --- | --- |
| CLI and prompts | `src/main.rs`, `src/prompt.rs`, `src/tool.rs` |
| Workspace configuration, reference snapshots and Git | `src/workspace.rs`, `src/git.rs` |
| Task lifecycle, status and diagnostics | `src/tasks.rs`, `src/report.rs`, `src/report/` |
| Generated workspace and task documents | `templates/workspace/`, `templates/task/`, `src/templates.rs` |
| Official workspace skills | `templates/workspace/.agents/skills/`, `src/templates/skills.rs` |

The workspace guides are starting points for teams to edit. The bundled **wk-cli** skill documents the command interface. **wk-workspace-workflow** covers project selection, handoff and closure. Edit their source files alongside the corresponding CLI behavior. Cargo embeds them in the binary for both release channels. After upgrading the binary, a workspace can run `wk doctor` and explicitly refresh registered skills with `wk skills sync [skill]`.

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
