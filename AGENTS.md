# Agent guide for workit-rs

`wk` creates one Git worktree per participating project and groups them in a task directory. Implement workspace behavior in Rust. Use Node.js or shell for project scripts. The npm launcher delegates to the same binary. Both distribution channels use the package name `workit-rs`.

## Run from source

Use Rust 1.94 or later, Cargo, and Git. From the repository root, run `cargo run --locked -- <args>` or `just wk <args>`. Use `just wk --help` for the command inventory and `just wk <command> --help` for its arguments.

For a manual check, run `just wk init`, then `just wk doctor`. The checkout sets `WK_WORKSPACE` to the ignored `.wk-dev/` directory through `.cargo/config.toml`. To reproduce against a different directory, use `just wk init <dir>` followed by `just wk -w <dir> status --json`. Keep automated test data in temporary directories.

Keep the init wizard transactional. Validate the empty target and finish collecting choices before publishing any files. Exercise cancellation in the PTY smoke test. Use `just clean-dev` only to discard the entire development workspace, including its Git data.

## Change the implementation

| Area | Source and contracts |
| --- | --- |
| CLI | `src/main.rs` defines clap arguments and dispatch. `src/tool.rs` owns supported launchers and the picker shared by `init` and `config`. Store enabled tool IDs in `wk.yaml` under `tools`. `wk run <task> [--agent <tool>]` accepts only enabled launchers, supports `--codex` and `--claude`, and launches the tool in the task directory with inherited terminal I/O. Propagate its exit status. Check for active tasks before asking for a run target or tool. Direct an empty task list to `wk start`. Limit launch suggestions to enabled tools found on PATH. |
| Workspace | `src/workspace.rs` owns the CLI-managed `wk.yaml` manifest. `add` / `clone` requires a fresh name and destination, allows repeated URLs, and persists the name, URL, and concrete task base. `init` creates a Git root and committable guides and ignore files. `add` and `sync` check out the configured baseline at detached HEAD. Fetching for other commands keeps the reference files at the last sync. |
| Task lifecycle | Check for registered projects before prompting for a task name. Direct an empty workspace to `wk add <url>`. Each worktree uses the exact task name as its local branch, including any slashes. `tasks/<task>/` is the common editor and harness directory, with one worktree per project. Group and task paths must not overlap. Resume from `.wk/active.yaml`. Record final commits before removing worktrees, and write one `.wk/history/<id>.yaml` before dropping the active entry. Before closing, check every member for local changes, untracked files, and ignored files. Remove its worktrees and task directory, then prune empty parent groups. Preserve branches, sibling tasks, and archived records. |
| Git and reporting | `src/git.rs` invokes system Git. `src/report.rs` lists registered projects and active tasks. Named task status compares each member's recorded starting ref and commit to its live checkout. Report committed file changes separately from pending files. Keep reference clone diagnostics in `doctor` and retain the clean-clone check before `status --fetch`. |

`wk doctor` displays basic workspace information and diagnoses problems. In `src/report/doctor.rs`, report one result per project and per active task. Expand only failures and attach member worktree problems to their task. Report interrupted operations and invalid task records at workspace level. The stdout summary counts result rows. Use SKIP for unavailable prerequisites and a nonzero exit code for failures. Keep diagnostics read-only and reuse workspace validation.

Keep ordinary clones in `repos/` as read-only baseline snapshots. Show the actual snapshot SHA in project status. Apply Windows device-name checks only on Windows. Commands using a reference clone must check it for local changes. Project removal must also protect open tasks, external worktrees and commits absent from locally known origin branches.

## Template ownership

- Keep `templates/workspace/AGENTS.md` short and editable by teams. The workspace harness scopes and manages tasks through wk. It consults reference code only for a concrete question and leaves direct access to task directories to a fresh task harness.
- Maintain command semantics in the bundled `wk-cli` skill and coordination decisions in `wk-workspace-workflow`. Sources are under `templates/workspace/.agents/skills/`. Match each frontmatter name to its directory. Its description appears in the init picker. The build embeds every skill file for both distribution channels.
- `wk doctor` compares the complete registered skill directories with the running binary. Only `wk skills sync` explicitly replaces their contents. Keep user-authored workspace documents and sibling skills outside this operation. Normal workspace sync restores missing files and preserves edits.
- Edit both task documents in `templates/task/`. Render `{{task}}` and `{{projects}}` from the task record. Snapshot optional project descriptions and the chosen bases and commits before creating any worktree. Write missing documents during task creation and preserve them on resume.
- Maintain presets in `src/preset.rs` and task skill copying in `src/task_skills.rs`. Resolve inherited bases at task creation. Stage complete skill directories before recording the task, then publish the captured selection under the task's `.agents/skills/`. On retry use the task record and snapshot independently of the catalog or preset. Keep one-off wizard selections out of `wk.yaml`.

## Verify a change

- Exercise CLI behavior in `tests/cli.rs` using the built binary and temporary local Git repositories. Run a single scenario with `cargo test --locked --test cli <test_name>`.
- CI runs on Ubuntu, macOS and Windows. Run `just check` for formatting, Clippy, and tests. The equivalent commands are `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, and `cargo test --locked`.
- For interactive `init`, `add`, `clone`, `start`, `config`, or `run` changes, run `just smoke-interactive` on Linux or macOS with Node.js 20+ and `socat`. It builds the debug binary, drives the prompts against local repositories, and checks the selected commits in each worktree.
- For npm launcher or packaging changes, use Node.js 20 or later, npm, and Bash. Run `just smoke-npm` to build and exercise both local tarballs. To inspect the crate contents before committing, use `cargo package --locked --allow-dirty --list`.

## Documentation and release

Keep workit-rs user documentation in matched English (`NAME.md`) and Simplified Chinese (`NAME.zh-CN.md`) files with reciprocal relative links. Update both when commands or examples change. Generated workspaces have a single English README. Agent guides and bundled skills are also in English.

For a release, align the versions in `Cargo.toml`, `Cargo.lock`, `package.json`, and its optional dependencies. Run `node scripts/npm-package.mjs check`. Dispatch the Release workflow on the candidate commit's branch to exercise CI and install all seven packaged targets, including the musl builds in Alpine. A matching `v<version>` tag runs the same checks before publishing with `CARGO_REGISTRY_TOKEN` and `NPM_TOKEN` from the GitHub `release` environment.

The Release workflow extracts the seven standalone executables from the tested npm archives and verifies `SHA256SUMS` before publishing the binaries and `release-manifest.json` on the tagged GitHub Release. Exercise `scripts/publish-npm.mjs` with `--dry-run` against the built archives. To retry after changing publisher code, dispatch Release on the default branch with the original verified tag run's `npm_run_id`. Preserve the original tarballs and published tag.
