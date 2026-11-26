# Project workspace

This Git repository coordinates projects that share development tasks. After cloning it, run `wk setup` and choose personal settings. Select the projects you use, or follow all shared projects. Use `wk setup --local` to change this checkout's selection and `wk sync` to update it. Team maintainers can use `wk setup --shared` to edit shared tools, official skills and project descriptions. Run `wk status` for the full project inventory and active tasks.

Use a workspace session to scope and start a task. Once the projects are selected, `wk start` prints launch commands. Use one to open a **fresh session** for implementation. To review or close the task from the workspace, use `wk status <task>` or `wk close <task>`. Closing removes shared files in the task directory along with its worktrees.

For a damaged task, `wk close <task>` asks for confirmation before permanently deleting remaining task files and recording the damage in history. Find closed tasks by local start date under `.wk/history/`.

Commit the manifest, this README, team instructions and selected skills with the workspace. You can edit this file and `AGENTS.md` to describe the workspace's purpose and team conventions. Registered official skills can be refreshed with `wk sync --skills`. Place team-authored skills in their own directories.

## Reusable task setups

Keep workspace-session skills in `.agents/skills/`. Put a skill intended for task sessions in `task-skills/<id>/SKILL.md` with YAML frontmatter containing `name` (matching the directory) and `description`, followed by its instructions. Place scripts, references and assets beside it and commit the directory. Use regular files and directories for portable snapshots.

Run `wk preset add <name>` to save a recurring combination of projects, base overrides and skills in `wk.yaml`. Leave a project's override blank to inherit its configured base. Use `wk preset list`, `show`, `edit` and `rm` to manage presets.

- `wk start feature/login --preset api-web` uses the preset directly.
- `wk start feature/login --preset api-web --interactive` opens the wizard with the preset's defaults. Changes apply to this task.
- `wk start feature/login api web -D --skills api-contract,integration-tests` sets the selection explicitly.

Each selected directory is copied into the task's `.agents/skills/`. Use `wk status <task>` to see the recorded preset and skill IDs. Edits to catalog skills or presets apply to future tasks.
