use crate::git::{
    check_clone_removal, check_root, fetch_origin, git, git_path, git_with_progress,
    remote_branches,
};
use crate::report::check_reference;
use crate::tasks::Phase;
use anyhow::{Context, Result, bail, ensure};
use inquire::Text;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    pub projects: BTreeMap<String, Project>,
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub name: String,
    pub url: String,
    pub base: String,
}

pub struct Workspace(pub PathBuf);

pub fn name_ok(name: &str) -> Result<()> {
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

// Check individual components so a replaced managed directory cannot redirect cleanup.
pub fn child(parent: &Path, component: &str) -> Result<PathBuf> {
    let path = parent.join(component);
    match path.symlink_metadata() {
        Ok(meta) => ensure!(!meta.is_symlink(), "refusing symlink {}", path.display()),
        Err(e) if e.kind() == ErrorKind::NotFound => (),
        Err(e) => return Err(e.into()),
    }
    Ok(path)
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    serde_json::from_reader(File::open(path).with_context(|| format!("open {}", path.display()))?)
        .with_context(|| format!("parse {}", path.display()))
}

pub fn save_json(path: &Path, data: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(data)?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp = NamedTempFile::new_in(path.parent().context("missing parent directory")?)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

const MANIFEST: &str = "wk.yaml";

impl Config {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.version == 1,
            "unsupported workspace version {}",
            self.version
        );
        for (name, project) in &self.projects {
            name_ok(name)?;
            ensure!(
                project.name == *name,
                "project key {name} differs from its name {}",
                project.name
            );
            validate_url(&project.url)?;
            validate_base(&project.base)?;
        }
        Ok(())
    }

    fn save(&self, path: &Path) -> Result<()> {
        write_atomic(path, serde_saphyr::to_string(self)?.as_bytes())
    }
}

fn validate_url(url: &str) -> Result<()> {
    ensure!(
        !url.trim().is_empty()
            && !url.starts_with('-')
            && !url.contains("::")
            && !url.contains(['\n', '\r']),
        "invalid Git URL"
    );
    Ok(())
}

fn validate_base(base: &str) -> Result<()> {
    ensure!(
        !base.trim().is_empty() && !base.starts_with('-') && !base.contains(['\n', '\r']),
        "invalid base ref"
    );
    Ok(())
}

impl Workspace {
    pub fn init(dir: &Path) -> Result<()> {
        fs::create_dir_all(dir)?;
        let ws = Self(dir.canonicalize()?);
        let _lock = ws.lock()?;
        let config = child(&ws.0, MANIFEST)?;
        if config.exists() {
            ws.config()?;
        } else {
            Config {
                version: 1,
                projects: BTreeMap::new(),
            }
            .save(&config)?;
        }
        for dir in ["repos", "tasks"] {
            fs::create_dir_all(child(&ws.0, dir)?)?;
        }
        let ignore = child(&ws.0, ".gitignore")?;
        let old = match fs::read_to_string(&ignore) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        let missing: Vec<_> = ["/repos/", "/tasks/", "/.wk/"]
            .into_iter()
            .filter(|entry| !old.lines().any(|line| line == *entry))
            .collect();
        if !missing.is_empty() {
            let mut f = OpenOptions::new().create(true).append(true).open(ignore)?;
            if !old.is_empty() && !old.ends_with('\n') {
                writeln!(f)?;
            }
            writeln!(f, "{}", missing.join("\n"))?;
        }
        let guide = child(&ws.0, "AGENTS.md")?;
        if !guide.exists() {
            fs::write(
                guide,
                "# Workspace\n\nUse wk start to create a task. Open tasks/<task>/ in your editor or harness. Keep task resources there and edit each project's code in its worktree at tasks/<task>/<project>/. Read the project's instructions and run Git and tests there. Leave repos/ to wk.\n",
            )?;
        }
        println!("Workspace: {}", ws.0.display());
        Ok(())
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

    pub fn config(&self) -> Result<Config> {
        let path = child(&self.0, MANIFEST)?;
        let config: Config = serde_saphyr::from_str(&fs::read_to_string(&path)?)
            .with_context(|| format!("parse {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn repo(&self, name: &str) -> Result<PathBuf> {
        name_ok(name)?;
        child(&child(&self.0, "repos")?, name)
    }

    pub fn verify_repo(&self, name: &str, project: &Project) -> Result<PathBuf> {
        let repo = self.repo(name)?;
        ensure!(repo.is_dir(), "missing clone {name}; run wk sync {name}");
        ensure!(
            child(&repo, ".git")?.is_dir(),
            "expected an ordinary clone at {}",
            repo.display()
        );
        check_root(&repo)?;
        ensure!(
            git(&repo, &["config", "--get", "remote.origin.url"])? == project.url,
            "{name}: origin URL differs from {MANIFEST}"
        );
        Ok(repo)
    }

    pub fn verify_clean_repo(&self, name: &str, project: &Project) -> Result<PathBuf> {
        let repo = self.verify_repo(name, project)?;
        check_reference(&repo)?;
        Ok(repo)
    }

    fn clone_project(&self, project: &Project) -> Result<()> {
        let repo = self.repo(&project.name)?;
        let destination = git_path(&repo)?;
        fs::create_dir_all(repo.parent().unwrap())?;
        // Git accepts an existing empty directory. Reserve a new one exclusively.
        fs::create_dir(&repo)
            .with_context(|| format!("create new directory {}", repo.display()))?;
        let cloned = git_with_progress(
            &self.0,
            &[
                "clone",
                "--origin",
                "origin",
                "--",
                &project.url,
                &destination,
            ],
            format!("Cloning {}", project.name),
        );
        if let Err(err) = cloned {
            // Remove only our empty reservation; preserve any files left by Git.
            let _ = fs::remove_dir(&repo);
            return Err(err);
        }
        Ok(())
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

    pub fn add(&self, url: &str, directory: Option<String>, base: Option<String>) -> Result<()> {
        let interactive = directory.is_none() && base.is_none() && crate::prompt::interactive();
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
            Text::new("Project directory")
                .with_default(default_name)
                .prompt()?
                .trim()
                .to_owned()
        } else {
            directory.unwrap_or_else(|| default_name.into())
        };
        self.require_new_project(&self.config()?, &name)?;
        let base = match base {
            Some(base) => base,
            None => {
                let branches = remote_branches(&self.0, &url, interactive)?;
                let branch = if interactive {
                    crate::prompt::branch("Task base", branches)?
                } else {
                    branches[0].clone()
                };
                format!("origin/{branch}")
            }
        };
        validate_base(&base)?;

        // Another writer may have used the name while the user was answering.
        let _lock = self.lock()?;
        let mut config = self.config()?;
        self.require_new_project(&config, &name)?;
        let project = Project {
            name: name.clone(),
            url,
            base,
        };
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
        for task_name in self.task_names()? {
            let task = self.read_task(&task_name)?;
            ensure!(
                task.phase == Phase::Closed || !task.projects.contains_key(name),
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
        let names = if names.is_empty() {
            config.projects.keys().cloned().collect()
        } else {
            names.to_vec()
        };
        for name in &names {
            ensure!(config.projects.contains_key(name), "unknown project {name}");
            if self.repo(name)?.try_exists()? {
                self.verify_clean_repo(name, &config.projects[name])?;
            }
        }
        for name in names {
            let project = &config.projects[&name];
            if self.repo(&name)?.exists() {
                let repo = self.repo(&name)?;
                fetch_origin(&repo, &name)?;
            } else {
                self.clone_project(project)?;
            }
            println!("Synced {name}");
        }
        Ok(())
    }

    pub fn task_dir(&self, name: &str) -> Result<PathBuf> {
        name_ok(name)?;
        child(&child(&self.0, "tasks")?, name)
    }

    pub fn records(&self) -> Result<PathBuf> {
        child(&child(&self.0, ".wk")?, "tasks")
    }

    pub fn record(&self, name: &str) -> Result<PathBuf> {
        name_ok(name)?;
        child(&self.records()?, &format!("{name}.json"))
    }
}
