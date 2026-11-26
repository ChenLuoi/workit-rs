---
name: wk-cli
description: Look up wk commands, arguments, Git effects and recovery steps when managing a workspace.
---

# wk command reference

Run `wk <command> --help` for accepted values and aliases. Use `wk -w <dir> <command>` to target a workspace explicitly. Otherwise wk uses `WK_WORKSPACE` when set or discovers the nearest enclosing `wk.yaml`.

## Workspace and projects

- `wk init [dir]` creates a Git workspace ready for a first commit. The target must be missing or empty, including hidden entries. Confirm the displayed path, optionally enter the first project's URL, then select launchers and official skills. For scripts, use `--yes` with optional `--url`, `--base`, `--description`, `--tools codex,claude` and `--skills wk-cli,wk-workspace-workflow`. With `--yes`, tools and skills default to an empty selection.
- `wk add <url> [directory] [--base <ref>] [--description <text>]` (alias `clone`) registers a new project and checks out a read-only reference snapshot. An existing name or destination blocks the operation. A URL can be registered under multiple names. With only the URL in a terminal, accept or change the inferred name, select a remote branch by prefix, and optionally describe the project's role. Scripts default to the repository name and remote default branch. Supply an explicit base for an empty remote.
- After cloning a workspace, run `wk setup`. Choose personal or shared settings at the first prompt. Use `--local` or `--shared` to skip the scope prompt. Scripts supply the scope and settings explicitly.
- `wk setup --local api web` saves a fixed project selection in `.wk/local.yaml`. `--all-projects` follows current and future registrations. Editing the project selection prepares newly enabled and missing clones. Deselecting a project preserves its clone and open tasks. `--tools codex` selects a subset of shared tools and `--all-tools` restores inheritance.
- `wk setup --shared` edits tools, bundled workspace skill registrations and project descriptions in `wk.yaml`. Use `--tools codex,vscode`, `--skills wk-cli`, or `--project <name> --description <text>`. An empty description clears the field. Passing `--tools` or `--skills` without a value clears that selection. Registering skills installs missing files. Deregistration leaves existing files in place. Supported tool IDs are `codex`, `claude`, `agy`, `vscode` and `zed`.
- `wk sync [project...]` restores missing clones, fetches origin and refreshes the selected reference checkouts at their configured bases. A fixed commit stays pinned. Sync also restores missing workspace template files. If wk reports local changes in a reference clone, preserve the work before restoring its clean snapshot.
- A bare sync uses the local project selection. Explicit names apply to this invocation. Successful clones remain usable if another project fails. Fix the reported cause and retry with `wk sync <project>`.
- `wk remove <project>` (alias `rm`) unregisters the project and deletes its clean clone. If the clone is missing, wk removes the registration. Open tasks, additional worktrees, private submodule data and commits absent from locally known origin branches can block removal. A task branch retained after close may still contain unpushed work.

## Task lifecycle

Use `wk preset add <name>` or `wk preset edit <name>` in a terminal to save project membership, optional base overrides and task skills. An empty override inherits the registered project's base. `wk preset list` lists available combinations, `wk preset show <name>` prints the saved YAML, and `wk preset rm <name>` removes it. These definitions are shared through `wk.yaml`.

Use `wk start <task> --preset <name>` for an exact combination, or add `-i` / `--interactive` to adjust it once. With no explicit project list, the terminal wizard offers saved presets and a “Start from scratch” choice. Without a preset, scripts can use `wk start <task> <projects...> -D --skills <id>,<id>`. `-D` accepts the effective starting refs and `--no-fetch` uses cached refs. A named preset supplies both project and skill selection. For one-off changes use the wizard.

Source each task skill from `task-skills/<id>/SKILL.md` and its adjacent files. Selected directories are copied into the task's `.agents/skills/`. Use `wk doctor` to diagnose broken preset references and `wk status <task>` to see the effective members, starts and skills. Resume using the recorded task name even if its preset has since been deleted.

If no projects are registered, `wk start` prompts you to register the first one with `wk add <url>` before asking for a task name. If there are no active tasks, `wk run` directs you to `wk start`. Once a task is active, give its name to `wk run <task>`.

- `wk start` marks the task root with an empty bare Git repository and creates one worktree per selected project, each on a new branch named after the task. A slash in the name groups directories and is preserved in the branch. In a terminal, choose projects with Space and Enter and accept default bases or choose a branch per project. Scripts supply a task name and either projects or a preset. An existing project branch can block creation.
- `wk run <task> [--agent <tool>]` opens an enabled, locally available tool at the task root. `--codex` and `--claude` are shortcuts. Omit the tool in a terminal for the picker. Give the user one of the launch commands printed by `start` for a fresh task session.
- `wk path <task> [project]` prints the absolute path for an editor or launcher.
- `wk status [task] [--json] [--fetch]` lists registered projects and active tasks. With a task name it shows starting refs and commits, current branches, committed differences and pending files. It reads local state by default. `--fetch` refreshes origin refs while leaving reference checkouts in place.
- `wk close <task>` checks all member worktrees, removes them through Git and deletes the entire task directory, including shared notes and fixtures. Task branches and the local history record survive. Changed, untracked or ignored files in a member block closing, as do refs stored in the task root's bare repository. After an interrupted close, rerun the same command.
- For a damaged task root or missing member directory, close reports the damage and asks for confirmation. `wk close <task> --force` confirms recovery in scripts. Confirming removes the task's stale worktree registrations, permanently deletes its remaining files and records the reasons under `damage` in the archive.
- Closed task archives in `.wk/history/` use local `YYYY—MM—DD—HH—MM—SS` dates in filenames and in `started_at` and `closed_at`.

## Diagnostics and official skills

- `wk doctor` reports workspace information and checks registered clones, active tasks, directory layout and registered official skills. It expands failed checks, performs no repairs and exits nonzero if a check fails.
- `wk skills sync [skill]` restores one registered official skill or all of them from this wk binary. The whole selected skill directory belongs to wk, including supporting files. Sync replaces changes and removes extra files there. Keep team skills under separate names. After upgrading wk, review the resulting workspace Git diff and commit the intended update.
