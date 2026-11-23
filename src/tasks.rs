use crate::{
    git::{
        branch_exists, bytes, cached_origin_branches, fetch_origin, git, git_path, path_arg,
        validate_tree,
    },
    prompt,
    workspace::{Config, Workspace, child, name_ok, read_json, save_json},
};
use anyhow::{Context, Result, ensure};
use inquire::Confirm;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Deserialize, Serialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Creating,
    Active,
    Closing,
    Closed,
}

#[derive(Deserialize, Serialize)]
pub struct Member {
    pub base: String,
    pub commit: String,
}

#[derive(Deserialize, Serialize)]
pub struct Task {
    pub phase: Phase,
    pub projects: BTreeMap<String, Member>,
}

pub fn branch(task: &str) -> String {
    format!("wk/{task}")
}

fn shell_path(path: &Path) -> Result<String> {
    let value = path_arg(path)?;
    #[cfg(windows)]
    return Ok(format!("'{}'", value.replace('\'', "''"))); // PowerShell
    #[cfg(not(windows))]
    Ok(format!("'{}'", value.replace('\'', "'\\''")))
}

fn git_record<'a>(listing: &'a [u8], tree: &Path) -> Result<Option<&'a [u8]>> {
    let prefix = format!("worktree {}", git_path(tree)?);
    // Each porcelain -z record ends with an empty field.
    let mut rest = listing;
    while !rest.is_empty() {
        let end = rest
            .windows(2)
            .position(|w| w == b"\0\0")
            .unwrap_or(rest.len());
        let row = &rest[..end];
        if row.split(|b| *b == 0).next() == Some(prefix.as_bytes()) {
            return Ok(Some(row));
        }
        rest = rest.get(end + 2..).unwrap_or_default();
    }
    Ok(None)
}

fn check_clean(tree: &Path) -> Result<()> {
    ensure!(
        bytes(
            tree,
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--ignore-submodules=none"
            ]
        )?
        .is_empty(),
        "{} has uncommitted or untracked files",
        tree.display()
    );
    ensure!(
        bytes(
            tree,
            &[
                "ls-files",
                "--others",
                "--ignored",
                "--exclude-standard",
                "-z"
            ]
        )?
        .is_empty(),
        "{} contains ignored files; move or remove them before closing",
        tree.display()
    );
    Ok(())
}

impl Workspace {
    pub fn task_names(&self) -> Result<Vec<String>> {
        let mut names = Vec::new();
        let records = self.records()?;
        if records.try_exists()? {
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
        names.sort_unstable();
        Ok(names)
    }

    pub fn read_task(&self, name: &str) -> Result<Task> {
        let task: Task = read_json(&self.record(name)?)?;
        ensure!(!task.projects.is_empty(), "empty task {name}");
        for (name, member) in &task.projects {
            name_ok(name)?;
            ensure!(
                matches!(member.commit.len(), 40 | 64)
                    && member.commit.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid recorded commit for {name}"
            );
        }
        Ok(task)
    }

    fn choose_start_bases(
        &self,
        name: &str,
        config: &Config,
        projects: &mut Vec<String>,
        no_fetch: bool,
        use_defaults: bool,
    ) -> Result<BTreeMap<String, Member>> {
        let interactive = prompt::interactive();
        if projects.is_empty() {
            ensure!(
                !config.projects.is_empty(),
                "no registered projects; run wk add first"
            );
            ensure!(
                interactive,
                "specify projects: wk start {name} <project>... (interactive selection requires a terminal)"
            );
            *projects = prompt::projects(config.projects.keys().cloned().collect())?;
        }
        let selected: BTreeSet<_> = projects.iter().collect();
        ensure!(selected.len() == projects.len(), "duplicate project");
        for p in projects.iter() {
            ensure!(config.projects.contains_key(p), "unknown project {p}");
        }
        if !interactive || use_defaults {
            return Ok(BTreeMap::new());
        }
        let defaults = projects
            .iter()
            .map(|p| format!("{p}: {}", config.projects[p].base))
            .collect::<Vec<_>>()
            .join(", ");
        if Confirm::new("Use configured default bases?")
            .with_default(true)
            .with_help_message(&defaults)
            .prompt()?
        {
            return Ok(BTreeMap::new());
        }
        let mut choices = BTreeMap::new();
        {
            // Protect fetches and snapshot refs, then release the lock before prompting.
            let _lock = self.lock()?;
            let mut repos = BTreeMap::new();
            for p in selected {
                let repo = self.verify_clean_repo(p, &config.projects[p])?;
                ensure!(
                    !branch_exists(&repo, &branch(name))?,
                    "{p}: branch wk/{name} already exists"
                );
                repos.insert(p, repo);
            }
            for (p, repo) in repos {
                if !no_fetch {
                    fetch_origin(&repo, p)?;
                }
                let refs = cached_origin_branches(&repo)?;
                ensure!(
                    !refs.is_empty(),
                    "{p}: no cached origin branches; run wk sync {p} or use --default"
                );
                choices.insert(p, refs);
            }
        }
        let mut members = BTreeMap::new();
        for (p, refs) in choices {
            let mut names: Vec<_> = refs.keys().cloned().collect();
            if let Some(default) = config.projects[p].base.strip_prefix("origin/")
                && let Some(i) = names.iter().position(|b| b == default)
            {
                names[..=i].rotate_right(1);
            }
            let chosen = prompt::branch(&format!("Task base for {p}"), names)?;
            members.insert(
                p.clone(),
                Member {
                    base: format!("origin/{chosen}"),
                    commit: refs[&chosen].clone(),
                },
            );
        }
        Ok(members)
    }

    pub fn start(
        &self,
        name: &str,
        mut projects: Vec<String>,
        no_fetch: bool,
        use_defaults: bool,
    ) -> Result<()> {
        let dir = self.task_dir(name)?;
        let record = self.record(name)?;
        let branch = branch(name);
        let snapshot = self.config()?;
        let creating = !record.try_exists()?;
        let chosen = if creating {
            ensure!(
                !dir.try_exists()?,
                "task directory already exists: {}",
                dir.display()
            );
            self.choose_start_bases(name, &snapshot, &mut projects, no_fetch, use_defaults)?
        } else {
            BTreeMap::new()
        };

        let _lock = self.lock()?;
        let config = self.config()?;
        let mut task = if creating {
            ensure!(
                !record.try_exists()? && !dir.try_exists()?,
                "task {name} appeared while preparing it; rerun wk start {name}"
            );
            Task {
                phase: Phase::Creating,
                projects: chosen,
            }
        } else {
            let task = self.read_task(name)?;
            ensure!(
                matches!(task.phase, Phase::Creating | Phase::Active),
                "task {name} is closing or archived"
            );
            if !projects.is_empty() {
                let selected: BTreeSet<_> = projects.iter().collect();
                ensure!(selected.len() == projects.len(), "duplicate project");
                ensure!(
                    task.projects.keys().eq(selected),
                    "task {name} has a different set of projects"
                );
            }
            projects = task.projects.keys().cloned().collect();
            task
        };
        let mut repos = BTreeMap::new();
        for p in &projects {
            let spec = config
                .projects
                .get(p)
                .with_context(|| format!("unknown project {p}"))?;
            let repo = self.verify_clean_repo(p, spec)?;
            if creating {
                ensure!(
                    snapshot.projects.get(p) == Some(spec),
                    "{p}: configuration changed while preparing the task; retry"
                );
                ensure!(
                    !branch_exists(&repo, &branch)?,
                    "{p}: branch {branch} already exists"
                );
            }
            repos.insert(p.clone(), repo);
        }
        if creating {
            for (p, repo) in &repos {
                if let Some(member) = task.projects.get(p) {
                    git(
                        repo,
                        &["cat-file", "-e", &format!("{}^{{commit}}", member.commit)],
                    )
                    .with_context(|| {
                        format!("{p}: selected commit is no longer available; retry")
                    })?;
                    continue;
                }
                if !no_fetch {
                    fetch_origin(repo, p)?;
                }
                let base = &config.projects[p].base;
                let commit = git(
                    repo,
                    &[
                        "rev-parse",
                        "--verify",
                        "--end-of-options",
                        &format!("{base}^{{commit}}"),
                    ],
                )
                .with_context(|| format!("{p}: cannot resolve base {base}"))?;
                task.projects.insert(
                    p.clone(),
                    Member {
                        base: base.clone(),
                        commit,
                    },
                );
            }
            fs::create_dir_all(self.records()?)?;
            save_json(&record, &task)?;
        }
        if task.phase == Phase::Creating {
            fs::create_dir_all(&dir)?;
        }
        for (p, member) in &task.projects {
            let repo = &repos[p];
            let tree = child(&dir, p)?;
            if tree.exists() {
                validate_tree(repo, &tree, &branch)?;
            } else {
                ensure!(
                    task.phase == Phase::Creating,
                    "missing active worktree {}; run wk doctor",
                    tree.display()
                );
                if branch_exists(repo, &branch)? {
                    ensure!(
                        git(repo, &["rev-parse", &format!("refs/heads/{branch}")])?
                            == member.commit,
                        "{p}: existing task branch has moved"
                    );
                    git(repo, &["worktree", "add", "--", &git_path(&tree)?, &branch])?;
                } else {
                    git(
                        repo,
                        &[
                            "worktree",
                            "add",
                            "--no-track",
                            "-b",
                            &branch,
                            "--",
                            &git_path(&tree)?,
                            &member.commit,
                        ],
                    )?;
                }
            }
        }
        if task.phase == Phase::Creating {
            let guide = child(&dir, "AGENTS.md")?;
            if !guide.exists() {
                let members = task.projects.keys().cloned().collect::<Vec<_>>().join(", ");
                fs::write(
                    guide,
                    format!(
                        "# Task {name}\n\nProjects: {members}.\n\nUse this directory as the working directory for all harnesses and editors. Keep task briefs, fixtures and other shared resources here. Each project directory is a separate Git worktree: follow its instructions, run its checks and commit there. Closing the task removes this entire directory.\n"
                    ),
                )?;
            }
            task.phase = Phase::Active;
            save_json(&record, &task)?;
        }
        let invoked_as = PathBuf::from(std::env::args_os().next().context("missing argv[0]")?);
        let mut prefix = if invoked_as.components().count() > 1 {
            let exe = shell_path(&std::env::current_exe()?)?;
            if cfg!(windows) {
                format!("& {exe}")
            } else {
                exe
            }
        } else {
            "wk".into()
        };
        if std::env::var_os("WK_WORKSPACE").is_some()
            || !Workspace::find(&std::env::current_dir()?).is_ok_and(|ws| ws.0 == self.0)
        {
            prefix.push_str(&format!(" --workspace {}", shell_path(&self.0)?));
        }
        println!(
            "{}\n{prefix} run {name} -- codex\n{prefix} run {name} -- claude",
            dir.display()
        );
        Ok(())
    }

    pub fn active_path(&self, name: &str, project: Option<&str>) -> Result<PathBuf> {
        let task = self.read_task(name)?;
        ensure!(task.phase == Phase::Active, "task {name} is not active");
        let dir = self.task_dir(name)?;
        let config = self.config()?;
        for p in task.projects.keys() {
            let spec = config
                .projects
                .get(p)
                .with_context(|| format!("unregistered project {p}"))?;
            validate_tree(
                &self.verify_clean_repo(p, spec)?,
                &child(&dir, p)?,
                &branch(name),
            )?;
        }
        if let Some(p) = project {
            ensure!(
                task.projects.contains_key(p),
                "{p} does not belong to {name}"
            );
            return child(&dir, p);
        }
        Ok(dir)
    }

    pub fn close(&self, name: &str) -> Result<()> {
        let _lock = self.lock()?;
        let mut task = self.read_task(name)?;
        if task.phase == Phase::Closed {
            println!("Already closed: {name}");
            return Ok(());
        }
        let config = self.config()?;
        let dir = self.task_dir(name)?;
        let mut removals = Vec::new();
        for p in task.projects.keys() {
            let tree = child(&dir, p)?;
            let spec = config
                .projects
                .get(p)
                .with_context(|| format!("unregistered project {p}"))?;
            let repo = self.verify_clean_repo(p, spec)?;
            let listing = bytes(&repo, &["worktree", "list", "--porcelain", "-z"])?;
            let row = git_record(&listing, &tree)?;
            if !tree.exists() && row.is_none() && task.phase != Phase::Active {
                continue;
            }
            let row =
                row.with_context(|| format!("worktree is not registered: {}", tree.display()))?;
            ensure!(
                !row.split(|b| *b == 0)
                    .any(|f| f == b"locked" || f.starts_with(b"locked ")),
                "locked worktree: {}",
                tree.display()
            );
            validate_tree(&repo, &tree, &branch(name))?;
            check_clean(&tree)?;
            removals.push((repo, tree));
        }
        task.phase = Phase::Closing;
        let record = self.record(name)?;
        save_json(&record, &task)?;
        for (repo, tree) in removals {
            check_clean(&tree)?;
            git(&repo, &["worktree", "remove", "--", &git_path(&tree)?])?;
        }
        // A retry can arrive after the directory was removed but before the record was saved.
        if dir.try_exists()? {
            fs::remove_dir_all(&dir)
                .with_context(|| format!("remove task directory {}", dir.display()))?;
        }
        task.phase = Phase::Closed;
        save_json(&record, &task)?;
        println!("Closed {name}. Task directory removed; project branches retained.");
        Ok(())
    }
}
