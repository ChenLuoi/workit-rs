pub(crate) mod doctor;
mod render;

use crate::{
    git::{bytes, check_root, check_tree_repository, fetch_origin, git},
    tasks::{Member, Phase, Task},
    workspace::{Config, Project, Workspace, child},
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{collections::BTreeMap, path::Path};

#[derive(Default, Serialize)]
struct Checkout {
    branch: String,
    commit: Option<String>,
    upstream: Option<String>,
    ahead: Option<usize>,
    behind: Option<usize>,
    staged: usize,
    modified: usize,
    untracked: usize,
    conflicts: usize,
    #[serde(skip)]
    ignored: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    files: Option<Vec<ChangedFile>>,
}

#[derive(Serialize)]
struct ChangedFile {
    status: String,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_path: Option<String>,
}

impl ChangedFile {
    fn new(status: &[u8], path: &[u8], original_path: Option<&[u8]>) -> Self {
        Self {
            status: String::from_utf8_lossy(status).into_owned(),
            path: String::from_utf8_lossy(path).into_owned(),
            original_path: original_path.map(|s| String::from_utf8_lossy(s).into_owned()),
        }
    }
}

#[derive(PartialEq)]
enum CheckoutView {
    Reference,
    Summary,
    Detail,
}

fn checkout(dir: &Path, view: CheckoutView) -> Result<Checkout> {
    let mut state = Checkout {
        files: (view == CheckoutView::Detail).then(Vec::new),
        ..Checkout::default()
    };
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
    if view == CheckoutView::Reference {
        args.push("--ignored=matching");
    }
    let output = bytes(dir, &args)?;
    let mut rows = output.split(|b| *b == 0);
    while let Some(row) = rows.next() {
        if let Some(oid) = row.strip_prefix(b"# branch.oid ") {
            if oid != b"(initial)" {
                state.commit = Some(std::str::from_utf8(oid)?.to_owned());
            }
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
            // Porcelain v2 puts the path after a fixed number of space-delimited
            // fields. With -z a rename's original path is the next record.
            let (status, fields, original) = match row.first() {
                Some(b'1' | b'2') => {
                    state.staged += usize::from(row[2] != b'.');
                    state.modified += usize::from(row[3] != b'.');
                    if row[0] == b'2' {
                        (
                            &row[2..4],
                            10,
                            Some(rows.next().context("missing rename source")?),
                        )
                    } else {
                        (&row[2..4], 9, None)
                    }
                }
                Some(b'u') => {
                    state.conflicts += 1;
                    (&row[2..4], 11, None)
                }
                Some(b'?') => {
                    state.untracked += 1;
                    (b"??".as_slice(), 2, None)
                }
                Some(b'!') => {
                    state.ignored += 1;
                    continue;
                }
                _ => continue,
            };
            if let Some(files) = &mut state.files {
                let path = row.splitn(fields, |b| *b == b' ').last().unwrap();
                files.push(ChangedFile::new(status, path, original));
            }
        }
    }
    Ok(state)
}

fn reference_branch(state: &Checkout) -> Result<()> {
    ensure!(
        state.commit.is_none() || state.branch == "(detached)",
        "reference clone is on {}, expected a detached snapshot",
        state.branch
    );
    Ok(())
}

fn check_reference_head(dir: &Path, head: Option<&str>) -> Result<()> {
    let saved = git(
        dir,
        &[
            "for-each-ref",
            "--format=%(objectname)",
            "refs/wk/snapshot",
            "refs/wk/next-snapshot",
        ],
    )?;
    ensure!(
        match head {
            None => saved.is_empty(),
            Some(commit) => saved.lines().any(|s| s == commit),
        },
        "reference HEAD differs from the saved snapshot"
    );
    Ok(())
}

pub(crate) fn check_reference(dir: &Path) -> Result<()> {
    let check = || -> Result<()> {
        let state = checkout(dir, CheckoutView::Reference)?;
        ensure!(
            state.staged + state.modified + state.untracked + state.conflicts == 0,
            "reference clone has local changes or untracked files"
        );
        ensure!(state.ignored == 0, "reference clone contains ignored files");
        reference_branch(&state)?;
        check_reference_head(dir, state.commit.as_deref())?;
        Ok(())
    };
    check().with_context(|| {
        format!(
            "{}: preserve your work and restore the reference clone. Use wk start for development",
            dir.display()
        )
    })
}

fn committed_changes(tree: &Path, start: &str, head: &str) -> Result<Vec<ChangedFile>> {
    let mut files = Vec::new();
    if start == head {
        return Ok(files);
    }
    let output = bytes(
        tree,
        &[
            "diff",
            "--name-status",
            "-z",
            "--find-renames",
            "--ignore-submodules=none",
            start,
            head,
            "--",
        ],
    )?;
    let mut fields = output.split(|b| *b == 0);
    while let Some(status) = fields.next().filter(|s| !s.is_empty()) {
        let first = fields.next().context("missing diff path")?;
        let (path, original) = if matches!(status.first(), Some(b'R' | b'C')) {
            (
                fields.next().context("missing diff destination")?,
                Some(first),
            )
        } else {
            (first, None)
        };
        files.push(ChangedFile::new(status, path, original));
    }
    Ok(files)
}

#[derive(Serialize)]
struct ProjectStatus {
    #[serde(flatten)]
    project: Project,
    commit: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    disabled_locally: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Default, Serialize)]
struct MemberStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    start: Option<Member>,
    #[serde(flatten)]
    current: Option<Checkout>,
    #[serde(skip_serializing_if = "Option::is_none")]
    committed_changes: Option<Vec<ChangedFile>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize)]
struct Report {
    projects: BTreeMap<String, ProjectStatus>,
    tasks: BTreeMap<String, Task<MemberStatus>>,
}

impl Workspace {
    fn task_status(
        &self,
        config: &Config,
        name: &str,
        task: Task,
        detail: bool,
    ) -> Task<MemberStatus> {
        let mut projects = BTreeMap::new();
        for (p, member) in task.projects {
            let mut status = MemberStatus::default();
            let result = (|| -> Result<()> {
                let spec = config
                    .projects
                    .get(&p)
                    .with_context(|| format!("unregistered project {p}"))?;
                let repo = self.verify_repo(&p, spec)?;
                let tree = child(&self.task_dir(name)?, &p)?;
                check_root(&tree)?;
                check_tree_repository(&repo, &tree)?;
                let view = if detail {
                    CheckoutView::Detail
                } else {
                    CheckoutView::Summary
                };
                let current = status.current.insert(checkout(&tree, view)?);
                if detail && let Some(head) = &current.commit {
                    status.committed_changes =
                        Some(committed_changes(&tree, &member.commit, head)?);
                }
                ensure!(
                    current.branch == name,
                    "on {}, expected {name}",
                    current.branch
                );
                Ok(())
            })();
            status.start = detail.then_some(member);
            status.error = result.err().map(|e| format!("{e:#}"));
            projects.insert(p, status);
        }
        Task {
            phase: task.phase,
            skills: task.skills,
            preset: task.preset,
            started_at_unix_ms: task.started_at_unix_ms,
            closing_at_unix_ms: task.closing_at_unix_ms,
            history: task.history,
            projects,
        }
    }

    fn status_report(&self, selected: Option<&str>, fetch: bool) -> Result<Report> {
        let _lock = if fetch { Some(self.lock()?) } else { None };
        let mut config = self.config()?;
        let mut active = BTreeMap::new();
        if let Some(name) = selected {
            let task = self.active_index()?.into_task(name)?;
            ensure!(
                task.phase == Phase::Active,
                "task {name} is {}. Status is available for active tasks only",
                task.phase
            );
            config.projects.retain(|p, _| task.projects.contains_key(p));
            active.insert(name.to_owned(), task);
        } else {
            for (name, task) in self.active_index()?.tasks {
                if task.phase == Phase::Active {
                    active.insert(name, task);
                }
            }
        }
        let local = self.local_config()?;
        if fetch {
            let targets: Vec<_> = config
                .projects
                .iter()
                .filter(|(name, _)| selected.is_some() || local.includes(name))
                .collect();
            for &(name, spec) in &targets {
                self.verify_clean_repo(name, spec)?;
            }
            for (name, _) in targets {
                fetch_origin(&self.repo(name)?, name)?;
            }
        }
        let tasks = active
            .into_iter()
            .map(|(name, task)| {
                let status = self.task_status(&config, &name, task, selected.is_some());
                (name, status)
            })
            .collect();
        let mut projects = BTreeMap::new();
        for (name, project) in config.projects {
            let disabled_locally = !local.includes(&name);
            let head = (|| -> Result<String> {
                if disabled_locally && selected.is_none() && !self.repo(&name)?.try_exists()? {
                    return Ok(String::new());
                }
                let repo = self.verify_repo(&name, &project)?;
                git(&repo, &["rev-parse", "--revs-only", "HEAD"])
            })();
            let (commit, error) = match head {
                Ok(oid) => (Some(oid).filter(|b| !b.is_empty()), None),
                Err(e) => (None, Some(format!("{e:#}"))),
            };
            projects.insert(
                name,
                ProjectStatus {
                    project,
                    commit,
                    error,
                    disabled_locally,
                },
            );
        }
        Ok(Report { projects, tasks })
    }

    pub fn status(&self, selected: Option<&str>, json: bool, fetch: bool) -> Result<()> {
        let report = self.status_report(selected, fetch)?;
        if json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            report.print(selected);
        }
        Ok(())
    }
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Creating => "creating",
            Self::Active => "active",
            Self::Closing => "closing",
            Self::Recovering => "recovering",
        })
    }
}
