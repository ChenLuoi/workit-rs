mod close;

use crate::{
    git::{
        branch_exists, bytes, cached_origin_branches, fetch_origin, git, git_path, path_arg,
        resolve_base, validate_tree,
    },
    preset::{self, Preset},
    prompt, task_skills,
    workspace::{Config, Workspace, child, name_ok, publish_directory, save_yaml, task_name_ok},
};
use anyhow::{Context, Result, ensure};
use inquire::Confirm;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Deserialize, Serialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Creating,
    Active,
    Closing,
    Recovering,
}

#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    pub base: String,
    pub commit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_commit: Option<String>,
}

impl Member {
    pub fn check_commit(&self, name: &str) -> Result<()> {
        for oid in std::iter::once(&self.commit).chain(self.final_commit.as_ref()) {
            ensure!(
                matches!(oid.len(), 40 | 64) && oid.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid recorded commit for {name}"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Task<M = Member> {
    pub phase: Phase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(default)]
    pub skills: BTreeSet<String>,
    pub started_at_unix_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closing_at_unix_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<crate::history::ClosingHistory>,
    pub projects: BTreeMap<String, M>,
}

#[derive(clap::Args)]
pub struct StartOptions {
    /// Task and branch name, e.g. feature/login.
    task: Option<String>,
    /// Projects for a new task.
    projects: Vec<String>,
    /// Use a saved project, base and task skill selection.
    #[arg(long, conflicts_with_all = ["projects", "skills"])]
    preset: Option<String>,
    /// Task skills to copy from task-skills/ (comma-separated).
    #[arg(long, value_delimiter = ',')]
    skills: Option<Vec<String>>,
    /// Review and adjust the selections for this task in a terminal.
    #[arg(short, long)]
    interactive: bool,
    /// Accept the preset or project bases without asking.
    #[arg(short = 'D', long = "default")]
    use_defaults: bool,
    /// Use cached remote branches and commits without fetching.
    #[arg(long)]
    no_fetch: bool,
}

#[derive(Default)]
struct StartPlan {
    preset: Option<String>,
    selection: Preset,
    members: BTreeMap<String, Member>,
}

impl StartOptions {
    fn prepare(&self, ws: &Workspace, name: &str, config: &Config) -> Result<StartPlan> {
        let mut projects = self.projects.clone();
        let local = ws.local_config()?;
        let available: Vec<_> = config
            .projects
            .values()
            .filter(|p| local.includes(&p.name))
            .collect();
        ensure!(
            !available.is_empty(),
            "no projects enabled locally; run wk setup --local"
        );
        let mut preset = self.preset.clone();
        let mut selection = Preset::default();
        let tty = prompt::interactive();
        ensure!(
            !self.interactive || tty,
            "--interactive requires a terminal"
        );
        let wizard = tty && (preset.is_none() || self.interactive);
        if wizard && preset.is_none() && projects.is_empty() && !config.presets.is_empty() {
            preset = preset::choose(config)?;
        }
        if let Some(id) = &preset {
            selection = config
                .presets
                .get(id)
                .with_context(|| format!("unknown preset {id}"))?
                .clone();
            selection
                .check(ws, config)
                .with_context(|| format!("preset {id}"))?;
            projects = selection.projects.keys().cloned().collect();
        }
        for p in &projects {
            ensure!(config.projects.contains_key(p), "unknown project {p}");
        }
        local.require_projects(&projects)?;
        if wizard && (projects.is_empty() || self.interactive || preset.is_some()) {
            projects = prompt::projects(available, &projects, false)?;
        }
        ensure!(
            !projects.is_empty(),
            "specify projects or a preset: wk start {name} <project>... or --preset <name>"
        );
        let selected: BTreeSet<_> = projects.iter().collect();
        ensure!(selected.len() == projects.len(), "duplicate project");
        selection.projects.retain(|p, _| selected.contains(p));
        for p in &projects {
            selection.projects.entry(p.clone()).or_default();
        }
        let chosen = if wizard && !self.use_defaults {
            ws.choose_start_bases(name, config, &selection, self.no_fetch)?
        } else {
            BTreeMap::new()
        };
        if let Some(ids) = &self.skills {
            selection.skills = ids.iter().cloned().collect();
        } else if wizard {
            selection.skills = ws.choose_task_skills(&selection.skills)?;
        }
        Ok(StartPlan {
            preset,
            selection,
            members: chosen,
        })
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveIndex<T = Task> {
    pub tasks: BTreeMap<String, T>,
}

fn now_ms() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_millis()
        .try_into()?)
}

impl ActiveIndex {
    pub fn into_task(mut self, name: &str) -> Result<Task> {
        task_name_ok(name)?;
        self.tasks
            .remove(name)
            .with_context(|| format!("no open task {name}"))
    }

    fn save_task(&mut self, ws: &Workspace, name: &str, task: &Task) -> Result<()> {
        self.tasks.insert(name.to_owned(), task.clone());
        save_yaml(&ws.active_file()?, self)
    }
}

fn shell_path(path: &Path) -> Result<String> {
    let value = path_arg(path)?;
    #[cfg(windows)]
    {
        Ok(format!("'{}'", value.replace('\'', "''")))
    } // PowerShell
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

pub(crate) fn check_task_git_dir(dir: &Path) -> Result<()> {
    let dot_git = child(dir, ".git")?;
    ensure!(
        dot_git.is_dir()
            && git(
                dir,
                &["--git-dir=.git", "rev-parse", "--is-bare-repository"]
            )? == "true",
        "expected a bare Git repository at {}",
        dot_git.display()
    );
    Ok(())
}

impl Workspace {
    pub(crate) fn active_file(&self) -> Result<PathBuf> {
        child(&child(&self.0, ".wk")?, "active.yaml")
    }

    pub fn read_index<T: DeserializeOwned>(&self) -> Result<ActiveIndex<T>> {
        let path = self.active_file()?;
        if !path.try_exists()? {
            let root = child(&self.0, "tasks")?;
            if root.try_exists()? {
                for entry in fs::read_dir(root)? {
                    ensure!(
                        entry?.file_name() == ".gitignore",
                        "missing .wk/active.yaml with existing task files"
                    );
                }
            }
            return Ok(ActiveIndex {
                tasks: BTreeMap::new(),
            });
        }
        serde_saphyr::from_str(&fs::read_to_string(&path)?).context("parse .wk/active.yaml")
    }

    pub fn active_index(&self) -> Result<ActiveIndex> {
        let index: ActiveIndex = self.read_index()?;
        for (name, task) in &index.tasks {
            task_name_ok(name)?;
            for id in &task.skills {
                name_ok(id)?;
            }
            ensure!(!task.projects.is_empty(), "empty task {name}");
            for (project, member) in &task.projects {
                name_ok(project)?;
                member.check_commit(project)?;
            }
        }
        Ok(index)
    }

    pub fn ensure_active_index(&self) -> Result<()> {
        let index = self.active_index()?;
        let path = self.active_file()?;
        if !path.try_exists()? {
            save_yaml(&path, &index)?;
        }
        Ok(())
    }

    fn check_task_namespace(&self, name: &str, index: &ActiveIndex) -> Result<()> {
        for other in index.tasks.keys() {
            if other == name {
                continue;
            }
            // An ancestor cannot also be a task. Group spelling must agree even
            // on a case-insensitive filesystem.
            let difference = name.split('/').zip(other.split('/')).find(|(a, b)| a != b);
            ensure!(
                difference.is_some_and(|(a, b)| !a.eq_ignore_ascii_case(b)),
                "task name {name:?} conflicts with registered task {other:?}"
            );
        }
        Ok(())
    }

    fn choose_start_bases(
        &self,
        name: &str,
        config: &Config,
        selection: &Preset,
        no_fetch: bool,
    ) -> Result<BTreeMap<String, Member>> {
        let defaults = selection
            .projects
            .keys()
            .map(|p| format!("{p}: {}", selection.base(config, p)))
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
            for p in selection.projects.keys() {
                let repo = self.verify_clean_repo(p, &config.projects[p])?;
                ensure!(
                    !branch_exists(&repo, name)?,
                    "{p}: branch {name} already exists"
                );
                repos.insert(p, repo);
            }
            for (p, repo) in repos {
                if !no_fetch {
                    fetch_origin(&repo, p)?;
                }
                let refs = cached_origin_branches(&repo)?;
                let mut refs: BTreeMap<_, _> = refs
                    .into_iter()
                    .map(|(branch, commit)| {
                        let base = format!("origin/{branch}");
                        (branch, (base, commit))
                    })
                    .collect();
                let base = selection.base(config, p);
                // Tags, SHAs and local refs can also be the saved selection.
                if !refs.values().any(|(r, _)| r == base)
                    && let Ok(commit) = resolve_base(&repo, base)
                {
                    refs.insert(format!("{base} (configured)"), (base.to_owned(), commit));
                }
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
            if let Some(i) = names
                .iter()
                .position(|b| refs[b].0 == selection.base(config, p))
            {
                names[..=i].rotate_right(1);
            }
            let chosen = prompt::branch(&format!("Task base for {p}"), names)?;
            members.insert(
                p.clone(),
                Member {
                    base: refs[&chosen].0.clone(),
                    commit: refs[&chosen].1.clone(),
                    description: config.projects[p].description.clone(),
                    final_commit: None,
                },
            );
        }
        Ok(members)
    }

    pub fn start(&self, mut options: StartOptions) -> Result<()> {
        let snapshot = self.config()?;
        ensure!(
            !snapshot.projects.is_empty(),
            "no registered projects. Register a project first with wk add <url>."
        );
        let name = match options.task.take() {
            Some(name) => name,
            None => prompt::task_name()?,
        };
        let name = name.as_str();
        task_name_ok(name)?;
        let previous = self.active_index()?;
        let creating = !previous.tasks.contains_key(name);
        let plan = if creating {
            self.check_task_namespace(name, &previous)?;
            let dir = self.task_dir(name)?;
            ensure!(
                !dir.try_exists()?,
                "task directory already exists: {}",
                dir.display()
            );
            options.prepare(self, name, &snapshot)?
        } else {
            StartPlan::default()
        };

        let _lock = self.lock()?;
        let mut index = self.active_index()?;
        self.check_task_namespace(name, &index)?;
        let dir = self.task_dir(name)?;
        let config = self.config()?;
        let local = self.local_config()?;
        let projects;
        let mut task = if creating {
            projects = plan.selection.projects.keys().cloned().collect::<Vec<_>>();
            local.require_projects(&projects)?;
            if let Some(id) = &plan.preset {
                ensure!(
                    snapshot.presets.get(id) == config.presets.get(id),
                    "preset {id} changed while preparing the task; retry"
                );
            }
            ensure!(
                !index.tasks.contains_key(name) && !dir.try_exists()?,
                "task {name} appeared while preparing it; rerun wk start {name}"
            );
            let started_at_unix_ms = now_ms()?;
            Task {
                phase: Phase::Creating,
                preset: plan.preset,
                skills: plan.selection.skills.clone(),
                started_at_unix_ms,
                closing_at_unix_ms: None,
                history: None,
                projects: plan.members,
            }
        } else {
            let task = index
                .tasks
                .remove(name)
                .context("task closed while preparing it")?;
            ensure!(
                matches!(task.phase, Phase::Creating | Phase::Active),
                "task {name} is closing or archived"
            );
            ensure!(
                options.preset.is_none() || options.preset == task.preset,
                "task {name} was started with a different preset"
            );
            if let Some(ids) = options.skills {
                ensure!(
                    ids.into_iter().collect::<BTreeSet<_>>() == task.skills,
                    "task {name} has a different set of skills"
                );
            }
            if !options.projects.is_empty() {
                let selected: BTreeSet<_> = options.projects.iter().collect();
                ensure!(
                    selected.len() == options.projects.len(),
                    "duplicate project"
                );
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
                    !branch_exists(&repo, name)?,
                    "{p}: branch {name} already exists"
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
                if !options.no_fetch {
                    fetch_origin(repo, p)?;
                }
                let base = plan.selection.base(&config, p);
                let commit = resolve_base(repo, base).with_context(|| match &task.preset {
                    Some(id) => format!("preset {id}, project {p}: starting ref {base}"),
                    None => format!("project {p}: starting ref {base}"),
                })?;
                task.projects.insert(
                    p.clone(),
                    Member {
                        base: base.to_owned(),
                        commit,
                        description: config.projects[p].description.clone(),
                        final_commit: None,
                    },
                );
            }
            if !task.skills.is_empty() {
                let pending = self.pending_skills(name, task.started_at_unix_ms)?;
                self.stage_skills(&task.skills, &pending).with_context(|| {
                    format!(
                        "prepare skills for {}",
                        task.preset.as_deref().unwrap_or(name)
                    )
                })?;
                if let Err(error) = index.save_task(self, name, &task) {
                    task_skills::discard_pending(&pending)?;
                    return Err(error);
                }
            } else {
                index.save_task(self, name, &task)?;
            }
        }
        if task.phase == Phase::Creating {
            fs::create_dir_all(&dir)?;
            let dot_git = child(&dir, ".git")?;
            if !dot_git.try_exists()? {
                // Publish a complete discovery boundary so an interrupted start can retry.
                let staged = tempfile::tempdir_in(&dir)?;
                git(
                    staged.path(),
                    &["init", "--quiet", "--bare", "--template=", "."],
                )?;
                publish_directory(staged.path(), &dot_git)?;
            }
            if !task.skills.is_empty() {
                task_skills::install_snapshot(
                    &dir,
                    &self.pending_skills(name, task.started_at_unix_ms)?,
                    &task.skills,
                )?;
            }
        }
        check_task_git_dir(&dir)?;
        for (p, member) in &task.projects {
            let repo = &repos[p];
            let tree = child(&dir, p)?;
            if tree.exists() {
                validate_tree(repo, &tree, name)?;
            } else {
                ensure!(
                    task.phase == Phase::Creating,
                    "missing active worktree {}; run wk doctor",
                    tree.display()
                );
                if branch_exists(repo, name)? {
                    ensure!(
                        git(repo, &["rev-parse", &format!("refs/heads/{name}")])? == member.commit,
                        "{p}: existing task branch has moved"
                    );
                    git(repo, &["worktree", "add", "--", &git_path(&tree)?, name])?;
                } else {
                    git(
                        repo,
                        &[
                            "worktree",
                            "add",
                            "--no-track",
                            "-b",
                            name,
                            "--",
                            &git_path(&tree)?,
                            &member.commit,
                        ],
                    )?;
                }
            }
        }
        if task.phase == Phase::Creating {
            crate::templates::install_task_docs(&dir, name, &task)?;
            task.phase = Phase::Active;
            index.save_task(self, name, &task)?;
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
        println!("{}", dir.display());
        let enabled = local.enabled_tools(&config);
        let available = crate::tool::available_tools(&enabled);
        if enabled.is_empty() {
            println!("Enable tools with {prefix} setup");
        } else if available.is_empty() {
            println!("No enabled tool found on PATH. Check tool availability with {prefix} setup");
        }
        for tool in available {
            println!("{prefix} run {name} --agent {tool}");
        }
        Ok(())
    }

    pub fn active_path(&self, name: &str, task: &Task, project: Option<&str>) -> Result<PathBuf> {
        ensure!(task.phase == Phase::Active, "task {name} is not active");
        let dir = self.task_dir(name)?;
        check_task_git_dir(&dir)?;
        let config = self.config()?;
        for p in task.projects.keys() {
            let spec = config
                .projects
                .get(p)
                .with_context(|| format!("unregistered project {p}"))?;
            validate_tree(&self.verify_clean_repo(p, spec)?, &child(&dir, p)?, name)?;
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
}
