use crate::{
    git::{bytes, fetch_origin, git, validate_tree},
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
    #[serde(skip_serializing_if = "Option::is_none")]
    reference_issue: Option<String>,
}

fn checkout(dir: &Path, reference: bool) -> Result<Checkout> {
    let mut state = Checkout::default();
    let mut args = vec![
        "--no-optional-locks",
        "status",
        "--porcelain=v2",
        "--branch",
        "--ahead-behind",
        "-z",
        "--untracked-files=all",
        "--ignore-submodules=none",
    ];
    if reference {
        args.push("--ignored=matching");
    }
    let output = bytes(dir, &args)?;
    let mut ignored = false;
    let mut unborn = false;
    let mut rows = output.split(|b| *b == 0);
    while let Some(row) = rows.next() {
        if row == b"# branch.oid (initial)" {
            unborn = true;
        } else if let Some(s) = row.strip_prefix(b"# branch.head ") {
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
                Some(b'!') => ignored = true,
                _ => (),
            }
        }
    }
    if reference {
        let default_branch = git(
            dir,
            &[
                "for-each-ref",
                "--format=%(symref:lstrip=3)",
                "refs/remotes/origin/HEAD",
            ],
        )?;
        state.reference_issue =
            if state.staged + state.modified + state.untracked + state.conflicts > 0 {
                Some("reference clone has local changes or untracked files".into())
            } else if ignored {
                Some("reference clone contains ignored files".into())
            } else if !default_branch.is_empty() && default_branch != state.branch {
                Some(format!(
                    "reference clone is on {}, expected {default_branch}",
                    state.branch
                ))
            } else if !unborn
                && !git(
                    dir,
                    &[
                        "rev-list",
                        "--max-count=1",
                        "HEAD",
                        "--not",
                        "--remotes=origin",
                    ],
                )?
                .is_empty()
            {
                Some("reference HEAD contains a commit not on a locally known origin branch".into())
            } else {
                None
            };
    }
    Ok(state)
}

pub(crate) fn check_reference(dir: &Path) -> Result<()> {
    if let Some(issue) = checkout(dir, true)?.reference_issue {
        anyhow::bail!(
            "{}: {issue}; preserve your work and restore the reference clone. Use wk start for development",
            dir.display()
        );
    }
    Ok(())
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
                if let Some(issue) = &c.reference_issue {
                    write!(f, " | {issue}")?;
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
            self.task_names()?
        };
        let selected_task = selected.map(|t| self.read_task(t)).transpose()?;
        let projects = config.projects.iter().filter(|(p, _)| {
            selected_task
                .as_ref()
                .is_none_or(|t| t.projects.contains_key(*p))
        });
        if fetch {
            for (p, spec) in projects.clone() {
                self.verify_clean_repo(p, spec)?;
            }
        }
        for (p, spec) in projects {
            if fetch {
                fetch_origin(&self.repo(p)?, p)?;
            }
            let result = self
                .verify_repo(p, spec)
                .and_then(|repo| checkout(&repo, true));
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
                        checkout(&tree, false)
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
            match entry {
                Entry::Error { error } => issues.push(format!("project {p}: {error}")),
                Entry::Ok(checkout) => {
                    if let Some(issue) = &checkout.reference_issue {
                        issues.push(format!(
                            "project {p}: {issue}; use wk start for development"
                        ));
                    }
                }
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
        for dir in ["repos", "tasks"] {
            let root = child(&self.0, dir)?;
            if !root.is_dir() {
                issues.push(format!("missing {dir}/"));
                continue;
            }
            for entry in fs::read_dir(root)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if dir == "tasks"
                    && report
                        .tasks
                        .get(&name)
                        .is_some_and(|t| t.phase == Phase::Closed)
                {
                    issues.push(format!("directory for closed task: {dir}/{name}"));
                } else if !(if dir == "repos" {
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
