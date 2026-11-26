mod manifest;

use super::{CheckoutView, check_reference_head, checkout, reference_branch};
use crate::{
    git::{check_root, check_tree_branch, check_tree_repository, git},
    tasks::{Member, Phase, Task, check_task_git_dir},
    workspace::{Project, Workspace, child, name_ok, validate_base, validate_url},
};
use anyhow::{Result, ensure};
use console::style;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Default)]
struct Checks {
    passed: usize,
    failed: usize,
    skipped: usize,
}

// Keep independent failures together until their project or task is rendered.
#[derive(Default)]
struct Findings {
    problems: Vec<String>,
    blocked: Option<&'static str>,
}

impl Findings {
    fn check<T>(&mut self, label: &str, result: Result<T>) -> Option<T> {
        match result {
            Ok(value) => Some(value),
            Err(e) => {
                self.problems.push(format!("{label}  {e:#}"));
                None
            }
        }
    }

    fn include(&mut self, name: &str, findings: Self) {
        self.problems.extend(
            findings
                .problems
                .into_iter()
                .map(|line| format!("{name}: {line}")),
        );
        self.blocked = self.blocked.or(findings.blocked);
    }
}

fn inspect_project(
    ws: &Workspace,
    name: &str,
    project: Result<Project>,
    inspect_clone: bool,
    has_git: bool,
    findings: &mut Findings,
) -> Option<PathBuf> {
    let project = findings.check("Project fields", project);
    if let Some(p) = &project {
        findings.check("Project name", p.check_name(name));
        findings.check("Git URL", validate_url(&p.url));
        findings.check("Task base", validate_base(&p.base));
    }
    if !inspect_clone {
        findings.check("Project key", name_ok(name));
        findings.blocked = Some("disabled locally");
        return None;
    }
    // clone_directory also validates the project key before using it as a path.
    let dir = findings.check("Clone directory", ws.clone_directory(name))?;
    findings.check("Ordinary clone", Workspace::check_ordinary_clone(&dir))?;
    if !has_git {
        findings.blocked = Some("Git unavailable");
        return None;
    }
    findings.check("Checkout root", check_root(&dir))?;
    if let Some(p) = project {
        findings.check("Origin matches wk.yaml", Workspace::check_origin(&dir, &p));
    }
    if let Some(state) = findings.check(
        "Working tree status",
        checkout(&dir, CheckoutView::Reference),
    ) {
        for (label, count) in [
            ("Staged files", state.staged),
            ("Modified files", state.modified),
            ("Untracked files", state.untracked),
            ("Conflicts", state.conflicts),
            ("Ignored files", state.ignored),
        ] {
            if count > 0 {
                findings.problems.push(format!("{label}  {count}"));
            }
        }
        findings.check("Detached snapshot", reference_branch(&state));
        findings.check(
            "Snapshot commit",
            check_reference_head(&dir, state.commit.as_deref()),
        );
    }
    Some(dir)
}

fn inspect_worktree(
    dir: &Path,
    name: &str,
    project: &str,
    repo: Option<&Path>,
    has_git: bool,
    findings: &mut Findings,
) {
    let tree = child(dir, project).and_then(|tree| {
        ensure!(tree.is_dir(), "missing worktree {}", tree.display());
        Ok(tree)
    });
    let Some(tree) = findings.check("Worktree directory", tree) else {
        return;
    };
    if !has_git {
        findings.blocked = Some("Git unavailable");
        return;
    }
    if findings.check("Worktree root", check_root(&tree)).is_none() {
        return;
    }
    if let Some(repo) = repo {
        findings.check("Worktree repository", check_tree_repository(repo, &tree));
    } else {
        findings.blocked = Some("reference clone unavailable");
    }
    findings.check("Task branch", check_tree_branch(&tree, name));
    findings.check(
        "Readable worktree status",
        checkout(&tree, CheckoutView::Summary),
    );
}

fn inspect_task(
    ws: &Workspace,
    name: &str,
    task: Task<serde_json::Value>,
    projects: Option<&BTreeSet<String>>,
    repos: &BTreeMap<String, PathBuf>,
    has_git: bool,
) -> Findings {
    let mut findings = Findings::default();
    if task.projects.is_empty() {
        findings
            .problems
            .push("Task members  empty task record".into());
    }
    if task.phase != Phase::Active {
        findings
            .problems
            .push(format!("incomplete ({})", task.phase));
    }
    let dir = findings.check(
        "Task directory",
        ws.task_dir(name).and_then(|dir| {
            ensure!(dir.is_dir(), "missing tasks/{name}/");
            Ok(dir)
        }),
    );
    if let Some(dir) = dir.as_ref().filter(|_| has_git) {
        findings.check("Task Git directory", check_task_git_dir(dir));
    }
    for (p, raw) in task.projects {
        let mut member = Findings::default();
        let valid_name = member.check("Member name", name_ok(&p)).is_some();
        if let Some(record) = member.check(
            "Member fields",
            serde_json::from_value::<Member>(raw).map_err(Into::into),
        ) {
            member.check("Recorded commit ID", record.check_commit(&p));
        }
        match projects {
            Some(names) if !names.contains(&p) => member
                .problems
                .push("Project registration  unknown project".into()),
            None => member.blocked = Some("workspace configuration unavailable"),
            _ => (),
        }
        if let Some(dir) = dir.as_ref().filter(|_| valid_name) {
            inspect_worktree(
                dir,
                name,
                &p,
                repos.get(&p).map(PathBuf::as_path),
                has_git,
                &mut member,
            );
        }
        findings.include(&p, member);
    }
    findings
}

impl Checks {
    fn pass(&mut self, label: &str, detail: &str) {
        self.passed += 1;
        let separator = if detail.is_empty() { "" } else { "  " };
        println!("  {}  {label}{separator}{detail}", style("✓ PASS").green());
    }

    fn fail(&mut self, label: &str, detail: &str) {
        self.failed += 1;
        let separator = if detail.is_empty() { "" } else { "  " };
        println!("  {}  {label}{separator}{detail}", style("✗ FAIL").red());
    }

    fn skip(&mut self, label: &str, reason: &str) {
        self.skipped += 1;
        println!("  {}  {label}  {reason}", style("– SKIP").yellow());
    }

    fn check<T>(
        &mut self,
        label: &str,
        result: Result<T>,
        detail: impl FnOnce(&T) -> String,
    ) -> Option<T> {
        match result {
            Ok(value) => {
                self.pass(label, &detail(&value));
                Some(value)
            }
            Err(error) => {
                self.fail(label, &format!("{error:#}"));
                None
            }
        }
    }

    fn group(&mut self, label: &str, detail: &str, findings: Findings) {
        if !findings.problems.is_empty() {
            self.fail(label, detail);
            for problem in findings.problems {
                println!("          • {problem}");
            }
        } else if let Some(reason) = findings.blocked {
            self.skip(label, reason);
        } else {
            self.pass(label, detail);
        }
    }

    fn layout(&mut self, ws: &Workspace, dir: &str, known: Option<&BTreeSet<String>>) {
        let root = self.check(
            &format!("{dir}/ directory"),
            child(&ws.0, dir).and_then(|p| {
                ensure!(p.is_dir(), "missing {dir}/");
                Ok(p)
            }),
            |_| String::new(),
        );
        let label = format!("Registered paths in {dir}/");
        let (Some(root), Some(known)) = (root, known) else {
            self.skip(&label, "directory or registration list unavailable");
            return;
        };
        let groups: BTreeSet<_> = known
            .iter()
            .flat_map(|name| name.match_indices('/').map(|(end, _)| &name[..end]))
            .collect();
        let mut pending = vec![(root, String::new())];
        while let Some((path, prefix)) = pending.pop() {
            let label = format!("Registered paths in {dir}/{prefix}");
            let entries =
                fs::read_dir(&path).and_then(|it| it.collect::<std::io::Result<Vec<_>>>());
            let mut entries = match entries {
                Ok(entries) => entries,
                Err(e) => {
                    self.fail(&label, &e.to_string());
                    continue;
                }
            };
            if prefix.is_empty() {
                entries.retain(|entry| {
                    entry.file_name() != ".gitignore"
                        || !entry.file_type().is_ok_and(|t| t.is_file())
                });
            }
            entries.sort_by_key(|e| e.file_name());
            let mut problems = Vec::new();
            for entry in &entries {
                let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
                if dir == "tasks"
                    && groups.contains(name.as_str())
                    && entry.file_type().is_ok_and(|t| t.is_dir())
                {
                    pending.push((entry.path(), format!("{name}/")));
                } else if !known.contains(&name) {
                    problems.push(format!("unregistered path {dir}/{name}"));
                }
            }
            if problems.is_empty() {
                self.pass(&label, &format!("{} checked", entries.len()));
            } else {
                self.fail(&label, &problems.join(", "));
            }
        }
    }
}

pub(crate) fn run(cwd: &Path, workspace: Result<Workspace>) -> Result<()> {
    let mut checks = Checks::default();
    println!(
        "{}  ·  wk {}",
        style("Workspace information").bold(),
        env!("CARGO_PKG_VERSION")
    );
    let has_git = checks
        .check("Git executable", git(cwd, &["--version"]), String::clone)
        .is_some();
    let ws = checks.check("Workspace discovery", workspace, |ws| {
        ws.0.display().to_string()
    });
    if let Some(ws) = ws {
        let config = checks.check("wk.yaml schema", manifest::read(&ws), |_| String::new());
        let config = match config {
            Some(config) => checks.check(
                "Workspace format version",
                config.check_version().map(|()| config),
                |c| c.version.to_string(),
            ),
            None => {
                checks.skip("Workspace format version", "wk.yaml unavailable");
                None
            }
        };
        let local = match ws.local_config() {
            Ok(local) => local,
            Err(e) => {
                checks.fail(".wk/local.yaml", &format!("{e:#}"));
                Default::default()
            }
        };
        let mut records = Findings::default();
        let index = records.check(
            "Task record index",
            ws.active_file().and_then(|path| {
                ensure!(
                    path.is_file(),
                    "missing .wk/active.yaml; run wk setup --local or wk sync"
                );
                ws.read_index::<serde_json::Value>()
            }),
        );
        let tasks = index
            .as_ref()
            .map(|i| i.tasks.keys().cloned().collect::<BTreeSet<_>>());
        let mut parsed = BTreeMap::new();
        if let Some(index) = index {
            for (name, raw) in index.tasks {
                let record = serde_json::from_value::<Task<serde_json::Value>>(raw);
                if let Some(task) =
                    records.check(&format!("{name}: Task record"), record.map_err(Into::into))
                {
                    parsed.insert(name, task);
                }
            }
        }
        let needed: BTreeSet<_> = parsed.values().flat_map(|t| t.projects.keys()).collect();
        let mut repos = BTreeMap::new();
        if let Some(config) = &config {
            let mut findings = Findings::default();
            for name in &config.skills {
                if let Some(issues) =
                    findings.check(name, crate::templates::skill_differences(&ws, name))
                {
                    findings.problems.extend(issues);
                }
            }
            let detail = if findings.problems.is_empty() {
                format!("{} registered", config.skills.len())
            } else {
                "compare with this wk version; restore with wk skills sync".into()
            };
            checks.group("Official skills", &detail, findings);
            let mut findings = Findings::default();
            for (id, preset) in &config.presets {
                if findings
                    .check(id, name_ok(id).and_then(|()| preset.check(&ws, config)))
                    .is_none()
                {
                    continue;
                }
                for (name, selection) in &preset.projects {
                    if !local.includes(name) {
                        findings.blocked = Some("preset includes a locally disabled project");
                        continue;
                    }
                    let Ok(spec) = &config.projects[name] else {
                        findings.blocked = Some("project configuration unavailable");
                        continue;
                    };
                    if !has_git {
                        findings.blocked = Some("Git unavailable");
                        continue;
                    }
                    let base = selection.base.as_deref().unwrap_or(&spec.base);
                    findings.check(
                        &format!("{id}: {name} ({base})"),
                        ws.verify_repo(name, spec)
                            .and_then(|repo| crate::git::resolve_base(&repo, base)),
                    );
                }
            }
            checks.group(
                "Task presets",
                &format!("{} configured", config.presets.len()),
                findings,
            );
        } else {
            checks.skip("Official skills", "workspace configuration unavailable");
            checks.skip("Task presets", "workspace configuration unavailable");
        }
        let mut project_reports = BTreeMap::new();
        let projects = config.map(|config| {
            let names = config.projects.keys().cloned().collect::<BTreeSet<_>>();
            checks.pass("Project inventory", &format!("{} registered", names.len()));
            for (name, project) in config.projects {
                let mut findings = Findings::default();
                let inspect_clone = local.includes(&name) || needed.contains(&name);
                if let Some(dir) =
                    inspect_project(&ws, &name, project, inspect_clone, has_git, &mut findings)
                {
                    repos.insert(name.clone(), dir);
                }
                project_reports.insert(name, findings);
            }
            names
        });
        if projects.is_none() {
            checks.skip("Project inventory", "workspace configuration unavailable");
        }

        let mut active = BTreeMap::new();
        for (name, task) in parsed {
            let phase = task.phase;
            let findings = inspect_task(&ws, &name, task, projects.as_ref(), &repos, has_git);
            if phase == Phase::Active {
                active.insert(name.clone(), findings);
            } else {
                records.include(&name, findings);
            }
        }
        let active_count = tasks
            .as_ref()
            .map_or_else(String::new, |_| format!("{} active", active.len()));
        checks.group("Task inventory", &active_count, records);

        for (heading, reports) in [("Projects", project_reports), ("Active tasks", active)] {
            if !reports.is_empty() {
                println!("\n{}", style(heading).bold());
                for (name, findings) in reports {
                    checks.group(&name, "", findings);
                }
            }
        }
        println!("\n{}", style("Directory layout").bold());
        checks.layout(&ws, "repos", projects.as_ref());
        checks.layout(&ws, "tasks", tasks.as_ref());
    } else {
        for label in [
            "wk.yaml schema",
            "Workspace format version",
            "Official skills",
            "Task presets",
            "Project inventory",
            "Task inventory",
            "repos/ directory",
            "Registered paths in repos/",
            "tasks/ directory",
            "Registered paths in tasks/",
        ] {
            checks.skip(label, "workspace unavailable");
        }
    }
    println!(
        "\nSummary: {} passed, {} failed, {} skipped",
        checks.passed, checks.failed, checks.skipped
    );
    ensure!(
        checks.failed == 0,
        "{} diagnostic result(s) failed",
        checks.failed
    );
    Ok(())
}
