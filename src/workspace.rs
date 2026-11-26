use crate::git::{
    check_clone_removal, check_root, fetch_origin, git, git_path, git_with_progress,
    refresh_snapshot, remote_branches,
};
use crate::prompt;
use crate::report::check_reference;
use crate::templates;
use crate::tool::{Tool, select_tools};
use anyhow::{Context, Result, bail, ensure};
use inquire::{Confirm, Text, validator::Validation};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;

#[derive(Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config<P = Project> {
    pub version: u32,
    #[serde(default)]
    pub tools: BTreeSet<Tool>,
    #[serde(default, deserialize_with = "templates::deserialize_skills")]
    pub skills: BTreeSet<String>,
    pub projects: BTreeMap<String, P>,
    #[serde(default)]
    pub presets: BTreeMap<String, crate::preset::Preset>,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub name: String,
    pub url: String,
    pub base: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

pub struct Workspace(pub PathBuf);

impl std::fmt::Display for Project {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)?;
        if let Some(role) = &self.description {
            write!(f, " — {role}")?;
        }
        Ok(())
    }
}

pub fn name_ok(name: &str) -> Result<()> {
    ensure!(
        !name.eq_ignore_ascii_case("README.md"),
        "project name conflicts with the task README.md"
    );
    component_ok(name)
}

fn component_ok(name: &str) -> Result<()> {
    #[cfg(windows)]
    {
        let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
        let numbered = stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"));
        ensure!(
            !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !numbered.is_some_and(|n| matches!(n.as_bytes(), [b'1'..=b'9'])),
            "Windows reserves the name {name:?}"
        );
    }
    ensure!(
        name.len() <= 100
            && name.starts_with(|c: char| c.is_ascii_alphanumeric())
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            && !name.contains("..")
            && !name.ends_with('.')
            && !name.ends_with(".lock")
            && !name.eq_ignore_ascii_case("AGENTS.md"),
        "invalid name {name:?}: use letters, digits, dots, '-' or '_'"
    );
    Ok(())
}

pub fn task_name_ok(name: &str) -> Result<()> {
    ensure!(name != "HEAD", "Git reserves HEAD as a branch name");
    ensure!(name.len() <= 100, "task name exceeds 100 characters");
    name.split('/')
        .try_for_each(component_ok)
        .with_context(|| format!("invalid task name {name:?}"))
}

pub fn is_link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT, including junctions
    }
    #[cfg(not(windows))]
    meta.is_symlink()
}

// Check individual components so a replaced managed directory cannot redirect cleanup.
pub fn child(parent: &Path, component: &str) -> Result<PathBuf> {
    let path = parent.join(component);
    match path.symlink_metadata() {
        Ok(meta) => ensure!(!is_link(&meta), "refusing symlink {}", path.display()),
        Err(e) if e.kind() == ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    Ok(path)
}

pub fn save_yaml(path: &Path, data: &impl Serialize) -> Result<()> {
    write_atomic(path, serde_saphyr::to_string(data)?.as_bytes())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp = NamedTempFile::new_in(path.parent().context("missing parent directory")?)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub const MANIFEST: &str = "wk.yaml";

pub fn publish_directory(source: &Path, destination: &Path) -> Result<()> {
    // An empty directory that appears during the clone belongs to its creator.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use rustix::fs::{CWD, RenameFlags, renameat_with};
        renameat_with(CWD, source, CWD, destination, RenameFlags::NOREPLACE)
            .with_context(|| format!("publish directory at {}", destination.display()))?;
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        ensure!(
            !destination.try_exists()?,
            "directory already exists: {}",
            destination.display()
        );
        fs::rename(source, destination)?;
    }
    Ok(())
}

// Resolve the existing part of a new target before displaying it for confirmation.
fn init_target(dir: &Path) -> Result<PathBuf> {
    match dir.symlink_metadata() {
        Ok(_) => Ok(dir.canonicalize()?),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let parent = init_target(dir.parent().context("missing workspace parent")?)?;
            let path = match dir.file_name() {
                Some(name) => parent.join(name),
                None => parent.parent().unwrap_or(&parent).to_owned(),
            };
            Ok(path)
        }
        Err(e) => Err(e.into()),
    }
}

impl<P> Config<P> {
    pub fn check_version(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "unsupported workspace version {}",
            self.version
        );
        Ok(())
    }
}

impl Project {
    pub fn check_name(&self, key: &str) -> Result<()> {
        ensure!(
            self.name == key,
            "project key {key} differs from its name {}",
            self.name
        );
        Ok(())
    }
}

pub(crate) fn project_description(
    input: Option<String>,
    ask: bool,
    current: &str,
) -> Result<Option<String>> {
    let value = match input {
        Some(value) => value,
        None if ask => {
            eprintln!(
                "Describe this project's role and related projects. Leave it blank to skip or clear."
            );
            Text::new("Project description")
                .with_initial_value(current)
                .prompt()?
        }
        None => return Ok(None),
    };
    let value = value.trim();
    ensure!(
        !value.chars().any(char::is_control),
        "enter a single-line project description"
    );
    Ok((!value.is_empty()).then(|| value.to_owned()))
}

impl Config {
    fn validate(&self) -> Result<()> {
        self.check_version()?;
        for (name, project) in &self.projects {
            name_ok(name)?;
            project.check_name(name)?;
            validate_url(&project.url)?;
            validate_base(&project.base)?;
        }
        Ok(())
    }

    fn save(&self, path: &Path) -> Result<()> {
        save_yaml(path, self)
    }
}

pub fn validate_url(url: &str) -> Result<()> {
    ensure!(
        !url.trim().is_empty()
            && !url.starts_with('-')
            && !url.contains("::")
            && !url.contains(['\n', '\r']),
        "invalid Git URL"
    );
    Ok(())
}

pub fn validate_base(base: &str) -> Result<()> {
    ensure!(
        !base.trim().is_empty() && !base.starts_with('-') && !base.contains(['\n', '\r']),
        "invalid base ref"
    );
    Ok(())
}

fn check_init_dir(dir: &Path, allowed: &[&Path]) -> Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).with_context(|| format!("inspect {}", dir.display())),
    };
    for entry in entries {
        ensure!(
            allowed.contains(&entry?.path().as_path()),
            "target directory is not empty: {}",
            dir.display()
        );
    }
    Ok(())
}

impl Workspace {
    pub fn init(
        dir: &Path,
        url: Option<String>,
        base: Option<String>,
        description: Option<String>,
        yes: bool,
        tools: Option<Vec<Tool>>,
        skills: Option<Vec<String>>,
    ) -> Result<()> {
        let ws = Self(init_target(&std::path::absolute(dir)?)?);
        check_init_dir(&ws.0, &[])?;
        if !yes {
            ensure!(
                prompt::interactive(),
                "use --yes to initialize without a terminal; optionally add --url <URL>"
            );
            eprintln!("This directory will hold your project clones and task worktrees.");
            let question = format!("Initialize workspace in {}?", ws.0.display());
            ensure!(
                Confirm::new(&question).with_default(true).prompt()?,
                "initialization cancelled"
            );
        }
        let project = Self::first_project(url, base, description, yes)?;
        let tools = match tools {
            Some(tools) => tools.into_iter().collect(),
            None if yes => BTreeSet::new(),
            None => select_tools(None)?,
        };
        let skills = match skills {
            Some(skills) => skills.into_iter().collect(),
            None if yes => BTreeSet::new(),
            None => templates::select_skills(None)?,
        };
        let stage = prompt::progress("Preparing workspace directory", || {
            check_init_dir(&ws.0, &[])?;
            fs::create_dir_all(&ws.0)?;
            Ok(tempfile::Builder::new()
                .prefix(".wk-init-")
                .tempdir_in(&ws.0)?)
        })?;
        let staged = Self(stage.path().to_owned());
        let mut config = Config {
            version: 1,
            tools,
            skills,
            projects: BTreeMap::new(),
            presets: BTreeMap::new(),
        };
        if let Some(project) = project {
            staged.clone_project(&project)?;
            config.projects.insert(project.name.clone(), project);
        }
        prompt::progress("Writing workspace files", || {
            // Check again after network I/O. Publish the manifest under the workspace lock.
            check_init_dir(&ws.0, &[stage.path()])?;
            let _lock = ws.lock()?;
            check_init_dir(&ws.0, &[stage.path(), &ws.0.join(".wk")])?;
            let mut manifest = NamedTempFile::new_in(stage.path())?;
            manifest.write_all(serde_saphyr::to_string(&config)?.as_bytes())?;
            manifest.as_file().sync_all()?;
            manifest.persist_noclobber(ws.0.join(MANIFEST))?;
            // From this point a retry with sync can restore every workspace file and clone.
            ws.ensure_layout(&config)
                .and_then(|()| {
                    for name in config.projects.keys() {
                        publish_directory(&staged.repo(name)?, &ws.repo(name)?)?;
                    }
                    Ok(())
                })
                .context("workspace registered; run wk sync to finish setup")
        })?;
        println!("Workspace: {}", ws.0.display());
        for name in config.projects.keys() {
            println!("Registered {name}");
        }
        Ok(())
    }

    fn first_project(
        url: Option<String>,
        base: Option<String>,
        description: Option<String>,
        yes: bool,
    ) -> Result<Option<Project>> {
        let url = match url {
            Some(url) => url,
            None if yes => {
                ensure!(base.is_none(), "--base requires --url when using --yes");
                ensure!(
                    description.is_none(),
                    "--description requires a first project URL"
                );
                return Ok(None);
            }
            None => {
                eprintln!("Enter a Git URL, or leave it blank to add a project later with wk add.");
                let answer = Text::new("First project URL")
                    .with_validator(|value: &str| {
                        let value = value.trim();
                        Ok(if value.is_empty() || validate_url(value).is_ok() {
                            Validation::Valid
                        } else {
                            Validation::Invalid("Enter a Git URL or leave it blank".into())
                        })
                    })
                    .prompt()?;
                let url = answer.trim();
                if url.is_empty() {
                    ensure!(
                        description.is_none(),
                        "--description requires a first project URL"
                    );
                    return Ok(None);
                }
                url.to_owned()
            }
        };
        let interactive = !yes && base.is_none();
        let mut project = Self::choose_project(
            &std::env::current_dir()?,
            &url,
            None,
            base,
            interactive,
            name_ok,
        )?;
        project.description = project_description(description, !yes, "")?;
        Ok(Some(project))
    }

    fn ensure_layout(&self, config: &Config) -> Result<()> {
        if !child(&self.0, ".git")?.try_exists()? {
            git(&self.0, &["init", "--quiet", "."])?;
        }
        templates::install(&self.0, &config.skills)?;
        self.ensure_active_index()
    }

    pub fn at_root(dir: &Path) -> Result<Self> {
        ensure!(
            child(dir, MANIFEST)?.is_file(),
            "no {MANIFEST} in {}; run wk init",
            dir.display()
        );
        Ok(Self(dir.canonicalize()?))
    }

    pub fn find(dir: &Path) -> Result<Self> {
        let dir = dir
            .canonicalize()
            .with_context(|| format!("locate {}", dir.display()))?;
        for root in dir.ancestors() {
            if child(root, MANIFEST)?.is_file() {
                return Ok(Self(root.to_owned()));
            }
        }
        bail!(
            "no {MANIFEST} above {}; run wk init or use --workspace",
            dir.display()
        )
    }

    pub fn lock(&self) -> Result<File> {
        let state = child(&self.0, ".wk")?;
        fs::create_dir_all(&state)?;
        let lock = File::options()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(child(&state, "lock")?)?;
        lock.try_lock()
            .context("cannot lock this workspace for writing")?;
        Ok(lock)
    }

    pub fn manifest_text(&self) -> Result<String> {
        let path = child(&self.0, MANIFEST)?;
        fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))
    }

    pub fn config(&self) -> Result<Config> {
        let config: Config = serde_saphyr::from_str(&self.manifest_text()?)
            .with_context(|| format!("parse {}", self.0.join(MANIFEST).display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn repo(&self, name: &str) -> Result<PathBuf> {
        name_ok(name)?;
        child(&child(&self.0, "repos")?, name)
    }

    pub fn clone_directory(&self, name: &str) -> Result<PathBuf> {
        let repo = self.repo(name)?;
        ensure!(repo.is_dir(), "missing clone {name}; run wk sync {name}");
        Ok(repo)
    }

    pub fn check_ordinary_clone(repo: &Path) -> Result<()> {
        ensure!(
            child(repo, ".git")?.is_dir(),
            "expected an ordinary clone at {}",
            repo.display()
        );
        Ok(())
    }

    pub fn check_origin(repo: &Path, project: &Project) -> Result<()> {
        ensure!(
            git(repo, &["config", "--get", "remote.origin.url"])? == project.url,
            "{}: origin URL differs from {MANIFEST}",
            project.name
        );
        Ok(())
    }

    pub fn verify_repo(&self, name: &str, project: &Project) -> Result<PathBuf> {
        let repo = self.clone_directory(name)?;
        Self::check_ordinary_clone(&repo)?;
        check_root(&repo)?;
        Self::check_origin(&repo, project)?;
        Ok(repo)
    }

    pub fn verify_clean_repo(&self, name: &str, project: &Project) -> Result<PathBuf> {
        let repo = self.verify_repo(name, project)?;
        check_reference(&repo)?;
        Ok(repo)
    }

    fn clone_project(&self, project: &Project) -> Result<()> {
        let repo = self.repo(&project.name)?;
        fs::create_dir_all(repo.parent().unwrap())?;
        ensure!(
            !repo.try_exists()?,
            "directory already exists: {}",
            repo.display()
        );
        let stage = tempfile::Builder::new()
            .prefix(".wk-clone-")
            .tempdir_in(repo.parent().unwrap())?;
        git_with_progress(
            &self.0,
            &[
                "clone",
                "--no-checkout",
                "--origin",
                "origin",
                "--",
                &project.url,
                &git_path(stage.path())?,
            ],
            format!("Cloning {}", project.name),
        )?;
        refresh_snapshot(stage.path(), &project.base)?;
        publish_directory(stage.path(), &repo)
    }

    fn require_new_project(&self, config: &Config, name: &str) -> Result<()> {
        ensure!(
            !config.projects.contains_key(name),
            "project {name} is already registered"
        );
        let repo = self.repo(name)?;
        ensure!(
            !repo.try_exists()?,
            "directory already exists: {}",
            repo.display()
        );
        Ok(())
    }

    fn choose_project(
        lookup_dir: &Path,
        url: &str,
        directory: Option<String>,
        base: Option<String>,
        interactive: bool,
        check_name: impl FnOnce(&str) -> Result<()>,
    ) -> Result<Project> {
        let path = Path::new(url);
        let (url, local_name) = if path.exists() {
            (
                git_path(&path.canonicalize()?)?,
                path.file_name().and_then(|s| s.to_str()),
            )
        } else {
            (url.to_owned(), None)
        };
        validate_url(&url)?;
        let source = url.trim_end_matches('/');
        let source = source.strip_suffix("/.git").unwrap_or(source);
        let leaf = local_name
            .filter(|name| *name != ".git")
            .unwrap_or_else(|| source.rsplit(['/', ':']).next().unwrap_or(""));
        let default_name = leaf.strip_suffix(".git").unwrap_or(leaf);
        let name = if interactive {
            eprintln!("This name identifies the project and its directory under repos/.");
            Text::new("Project directory")
                .with_default(default_name)
                .prompt()?
                .trim()
                .to_owned()
        } else {
            directory.unwrap_or_else(|| default_name.into())
        };
        check_name(&name)?;
        let base = match base {
            Some(base) => base,
            None => {
                let branches = remote_branches(lookup_dir, &url, interactive)?;
                let branch = if interactive {
                    eprintln!("New tasks for this project will start from the selected branch.");
                    prompt::branch("Task base", branches)?
                } else {
                    branches[0].clone()
                };
                format!("origin/{branch}")
            }
        };
        validate_base(&base)?;
        Ok(Project {
            name,
            url,
            base,
            description: None,
        })
    }

    pub fn add(
        &self,
        url: &str,
        directory: Option<String>,
        base: Option<String>,
        description: Option<String>,
    ) -> Result<()> {
        let interactive = directory.is_none() && base.is_none() && prompt::interactive();
        let mut project =
            Self::choose_project(&self.0, url, directory, base, interactive, |name| {
                self.require_new_project(&self.config()?, name)
            })?;
        project.description = project_description(description, interactive, "")?;
        // Another writer may have used the name while the user was answering.
        let _lock = self.lock()?;
        let mut config = self.config()?;
        let name = project.name.clone();
        self.require_new_project(&config, &name)?;
        self.clone_project(&project)?;
        config.projects.insert(name.clone(), project);
        config.save(&child(&self.0, MANIFEST)?).with_context(|| {
            format!("cloned repos/{name} but could not register it in {MANIFEST}")
        })?;
        println!("Registered {name}");
        Ok(())
    }

    pub fn remove(&self, name: &str) -> Result<()> {
        let _lock = self.lock()?;
        let mut config = self.config()?;
        let spec = config
            .projects
            .remove(name)
            .with_context(|| format!("unknown project {name}"))?;
        for (task_name, task) in self.active_index()?.tasks {
            ensure!(
                !task.projects.contains_key(name),
                "project {name} is used by task {task_name}; close it with wk close {task_name} first"
            );
        }
        let repo = self.repo(name)?;
        let manifest = child(&self.0, MANIFEST)?;
        if repo.try_exists()? {
            self.verify_clean_repo(name, &spec)?;
            check_clone_removal(&repo)?;
            // Keep the clone recoverable until the manifest replacement succeeds.
            let stage = tempfile::Builder::new()
                .prefix(&format!("remove-{name}-"))
                .tempdir_in(child(&self.0, ".wk")?)?;
            let staged = stage.path().join(name);
            fs::rename(&repo, &staged)
                .with_context(|| format!("move {} for removal", repo.display()))?;
            if let Err(error) = config.save(&manifest) {
                if let Err(restore) = fs::rename(&staged, &repo) {
                    let _ = stage.keep();
                    bail!(
                        "{error:#}; restore failed ({restore}); clone preserved at {}",
                        staged.display()
                    );
                }
                return Err(error);
            }
            stage.close().with_context(|| {
                format!(
                    "removed {name} from the manifest; could not finish deleting {}",
                    staged.display()
                )
            })?;
        } else {
            config.save(&manifest)?;
        }
        println!("Removed {name}");
        Ok(())
    }

    pub fn sync(&self, names: &[String]) -> Result<()> {
        let _lock = self.lock()?;
        let config = self.config()?;
        let (names, skipped) = if names.is_empty() {
            let local = self.local_config()?;
            let enabled: Vec<_> = config
                .projects
                .keys()
                .filter(|p| local.includes(p))
                .cloned()
                .collect();
            let skipped = config.projects.len() - enabled.len();
            (enabled, skipped)
        } else {
            (names.to_vec(), 0)
        };
        for name in &names {
            ensure!(config.projects.contains_key(name), "unknown project {name}");
        }
        self.ensure_layout(&config)?;
        self.sync_projects(&config, &names, skipped)
    }

    // The caller holds the workspace lock and has validated the requested names.
    pub(crate) fn sync_projects(
        &self,
        config: &Config,
        names: &[String],
        skipped: usize,
    ) -> Result<()> {
        let mut failures = Vec::new();
        for name in names {
            match self.sync_project(name, &config.projects[name]) {
                Ok(()) => println!("Synced {name}"),
                Err(error) => failures.push(format!("{name}: {error:#}")),
            }
        }
        println!(
            "{} synced, {skipped} disabled locally, {} failed",
            names.len() - failures.len(),
            failures.len()
        );
        ensure!(
            failures.is_empty(),
            "sync failed for:\n{}",
            failures.join("\n")
        );
        Ok(())
    }

    fn sync_project(&self, name: &str, project: &Project) -> Result<()> {
        if !self.repo(name)?.try_exists()? {
            return self.clone_project(project);
        }
        let repo = self.verify_clean_repo(name, project)?;
        fetch_origin(&repo, name)?;
        self.verify_clean_repo(name, project)?;
        refresh_snapshot(&repo, &project.base)
    }

    pub fn task_dir(&self, name: &str) -> Result<PathBuf> {
        task_name_ok(name)?;
        name.split('/')
            .try_fold(child(&self.0, "tasks")?, |dir, part| child(&dir, part))
    }
}
