use crate::{
    git::{bytes, git, validate_tree},
    tasks::{Phase, branch},
    workspace::{Workspace, child},
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{collections::BTreeMap, fs, path::Path};

#[derive(Default, Serialize)]
struct Checkout {
    branch: String,
    upstream: Option<String>,
    ahead: Option<usize>,
    behind: Option<usize>,
    staged: usize,
    modified: usize,
    untracked: usize,
    conflicts: usize,
}

fn checkout(dir: &Path) -> Result<Checkout> {
    let mut state = Checkout::default();
    let output = bytes(
        dir,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
            "--ignore-submodules=none",
        ],
    )?;
    let mut rows = output.split(|b| *b == 0);
    while let Some(row) = rows.next() {
        if let Some(s) = row.strip_prefix(b"# branch.head ") {
            state.branch = String::from_utf8_lossy(s).into_owned();
        } else if let Some(s) = row.strip_prefix(b"# branch.upstream ") {
            state.upstream = Some(String::from_utf8_lossy(s).into_owned());
        } else if let Some(s) = row.strip_prefix(b"# branch.ab +") {
            let (ahead, behind) = std::str::from_utf8(s)?
                .split_once(" -")
                .context("invalid ahead/behind counts")?;
            state.ahead = Some(ahead.parse()?);
            state.behind = Some(behind.parse()?);
        } else {
            match row.first() {
                Some(b'1' | b'2') => {
                    state.staged += usize::from(row[2] != b'.');
                    state.modified += usize::from(row[3] != b'.');
                    if row[0] == b'2' {
                        rows.next();
                    } // original path of a rename/copy
                }
                Some(b'u') => state.conflicts += 1,
                Some(b'?') => state.untracked += 1,
                _ => (),
            }
        }
    }
    Ok(state)
}

#[derive(Serialize)]
#[serde(untagged)]
enum Entry {
    Ok(Checkout),
    Error { error: String },
}

impl From<Result<Checkout>> for Entry {
    fn from(result: Result<Checkout>) -> Self {
        match result {
            Ok(c) => Entry::Ok(c),
            Err(e) => Entry::Error {
                error: format!("{e:#}"),
            },
        }
    }
}

impl std::fmt::Display for Entry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error { error } => write!(f, "ERROR: {error}"),
            Self::Ok(c) => {
                write!(
                    f,
                    "{} | {} staged, {} modified, {} untracked, {} conflicts",
                    c.branch, c.staged, c.modified, c.untracked, c.conflicts
                )?;
                if let Some(u) = &c.upstream {
                    write!(f, " | {u}")?;
                    if let (Some(a), Some(b)) = (c.ahead, c.behind) {
                        write!(f, " +{a} -{b}")?;
                    }
                } else {
                    write!(f, " | no upstream")?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Serialize)]
struct TaskStatus {
    phase: Phase,
    projects: BTreeMap<String, Entry>,
}

#[derive(Default, Serialize)]
struct Report {
    projects: BTreeMap<String, Entry>,
    tasks: BTreeMap<String, TaskStatus>,
}

impl Workspace {
    fn inspect(&self, selected: Option<&str>, fetch: bool) -> Result<Report> {
        let _lock = if fetch { Some(self.lock()?) } else { None };
        let config = self.config()?;
        let mut report = Report::default();
        let tasks = if let Some(name) = selected {
            vec![name.to_owned()]
        } else {
            let mut names = Vec::new();
            let records = self.records()?;
            if records.exists() {
                for entry in fs::read_dir(records)? {
                    if let Some(name) = entry?
                        .file_name()
                        .to_str()
                        .and_then(|s| s.strip_suffix(".json"))
                    {
                        names.push(name.to_owned());
                    }
                }
            }
            names
        };
        let selected_task = selected.map(|t| self.read_task(t)).transpose()?;
        for (p, spec) in &config.projects {
            if selected_task
                .as_ref()
                .is_some_and(|t| !t.projects.contains_key(p))
            {
                continue;
            }
            let result = self.verify_repo(p, spec).and_then(|repo| {
                if fetch {
                    git(&repo, &["fetch", "origin"])?;
                }
                checkout(&repo)
            });
            report.projects.insert(p.clone(), result.into());
        }
        for name in tasks {
            let task = self.read_task(&name)?;
            let mut projects = BTreeMap::new();
            if task.phase != Phase::Closed {
                let dir = self.task_dir(&name)?;
                for p in task.projects.keys() {
                    let result = (|| {
                        let spec = config
                            .projects
                            .get(p)
                            .with_context(|| format!("unregistered project {p}"))?;
                        let repo = self.verify_repo(p, spec)?;
                        let tree = child(&dir, p)?;
                        validate_tree(&repo, &tree, &branch(&name))?;
                        checkout(&tree)
                    })();
                    projects.insert(p.clone(), result.into());
                }
            }
            report.tasks.insert(
                name,
                TaskStatus {
                    phase: task.phase,
                    projects,
                },
            );
        }
        Ok(report)
    }

    pub fn status(&self, selected: Option<&str>, json: bool, fetch: bool) -> Result<()> {
        let report = self.inspect(selected, fetch)?;
        if json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            println!("Projects ({})", report.projects.len());
            for (name, entry) in report.projects {
                println!("  {name}: {entry}");
            }
            println!("Tasks ({})", report.tasks.len());
            for (name, task) in report.tasks {
                println!("  {name} [{}]", task.phase);
                for (p, entry) in task.projects {
                    println!("    {p}: {entry}");
                }
            }
        }
        Ok(())
    }

    pub fn doctor(&self) -> Result<()> {
        println!("{}", git(&self.0, &["--version"])?);
        let report = self.inspect(None, false)?;
        let mut issues = Vec::new();
        for (p, entry) in &report.projects {
            if let Entry::Error { error } = entry {
                issues.push(format!("project {p}: {error}"));
            }
        }
        for (name, task) in &report.tasks {
            if matches!(task.phase, Phase::Creating | Phase::Closing) {
                issues.push(format!("task {name}: incomplete ({})", task.phase));
            }
            for (p, entry) in &task.projects {
                if let Entry::Error { error } = entry {
                    issues.push(format!("{name}/{p}: {error}"));
                }
            }
        }
        for dir in ["repos", "worktrees"] {
            let root = child(&self.0, dir)?;
            if !root.is_dir() {
                issues.push(format!("missing {dir}/"));
                continue;
            }
            for entry in fs::read_dir(root)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if !(if dir == "repos" {
                    report.projects.contains_key(&name)
                } else {
                    report.tasks.contains_key(&name)
                }) {
                    issues.push(format!("unregistered path {dir}/{name}"));
                }
            }
        }
        for issue in &issues {
            eprintln!("{issue}");
        }
        ensure!(issues.is_empty(), "{} workspace issue(s)", issues.len());
        println!("Workspace OK: {}", self.0.display());
        Ok(())
    }
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Creating => "creating",
            Self::Active => "active",
            Self::Closing => "closing",
            Self::Closed => "closed",
        })
    }
}
