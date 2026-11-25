---
name: wk-workspace-workflow
description: Scope a user request to registered projects, hand off a wk task to a fresh session, or close an existing task from the workspace.
---

# Coordinate work from the workspace

## Identify the task and its projects

If the user names a task, query `wk status <task>`. Otherwise use `wk status` to see active tasks and registered projects. Use project descriptions in `wk.yaml` to connect the requested change to the repositories that own it. Similar task names alone do not establish that the user wants to resume one.

When the project descriptions settle the scope, proceed with that selection. If one concrete ownership question remains, inspect only the relevant reference snapshot in `repos/<project>/`. For example, a search for the route that issues tokens can distinguish the API owner from its client. That checkout reflects its last sync. If two interpretations still imply different tasks, ask the user which outcome they want.

## Create or resume, then hand off

For a new task, choose its projects and the appropriate bases with `wk start`. For an existing task, use its recorded name and members. If creation was interrupted, repeat `wk start <task>` so wk can finish from the recorded commits.

For recurring joint work, inspect `wk preset list` and `wk preset show <name>`. Use a matching preset for the handoff. When the request changes one project's starting branch or needs a different integration skill, use `wk start <task> --preset <name> --interactive` to adjust this task. For a scripted handoff with explicit membership, select catalog skills through `--skills`. Report the actual task setup using `wk status <task>`.

Tell the user the task name, selected projects, and one launch command printed by wk. Carry the requested outcome and any unresolved scope question into the handoff. The user starts a fresh harness in the task directory. From the workspace session, use wk for task information and leave direct access to `tasks/` to that fresh session.

## Review and close

Answer progress questions with `wk status <task>`. Close when the user requests it. Task status describes member worktrees; shared files beside them are also removed by close. If their disposition matters, have the task session preserve them. Run `wk close <task>` from the workspace. If it reports files that block removal, give the user the affected project and the reported reason so the task session can resolve it.
