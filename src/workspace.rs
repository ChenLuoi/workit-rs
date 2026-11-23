use crate::git::{check_root, git, git_path};
use anyhow::{Context, Result, bail, ensure};
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

#[derive(Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
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
            validate_project(project)?;
        }
        Ok(())
    }

    fn save(&self, path: &Path) -> Result<()> {
        write_atomic(path, serde_saphyr::to_string(self)?.as_bytes())
    }
}

fn validate_project(project: &Project) -> Result<()> {
    ensure!(
        !project.url.trim().is_empty()
            && !project.url.starts_with('-')
            && !project.url.contains("::")
            && !project.url.contains(['\n', '\r']),
        "invalid Git URL"
    );
    if let Some(base) = &project.base {
        ensure!(
            !base.is_empty() && !base.starts_with('-'),
            "invalid base ref"
        );
    }
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
        for dir in ["repos", "worktrees", "tasks"] {
            fs::create_dir_all(child(&ws.0, dir)?)?;
        }
        let ignore = child(&ws.0, ".gitignore")?;
        let old = match fs::read_to_string(&ignore) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        let missing: Vec<_> = ["/repos/", "/worktrees/", "/.wk/"]
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
                "# Workspace\n\nWork in worktrees/<task>/<project>. Read each project's instructions before editing. Run Git and tests in the relevant project directory.\n",
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

    fn clone_project(&self, name: &str, project: &Project) -> Result<()> {
        let repo = self.repo(name)?;
        fs::create_dir_all(repo.parent().unwrap())?;
        git(
            &self.0,
            &[
                "clone",
                "--origin",
                "origin",
                "--",
                &project.url,
                &git_path(&repo)?,
            ],
        )?;
        Ok(())
    }

    pub fn add(&self, url: &str, name: Option<String>, base: Option<String>) -> Result<()> {
        let _lock = self.lock()?;
        let mut config = self.config()?;
        let name = name.unwrap_or_else(|| {
            url.trim_end_matches('/')
                .rsplit(['/', ':'])
                .next()
                .unwrap_or("")
                .trim_end_matches(".git")
                .into()
        });
        name_ok(&name)?;
        let url = if Path::new(url).exists() {
            git_path(&Path::new(url).canonicalize()?)?
        } else {
            url.into()
        };
        let project = Project { url, base };
        validate_project(&project)?;
        if let Some(old) = config.projects.get(&name) {
            ensure!(
                old == &project,
                "{name} already registered with different settings"
            );
        }
        if self.repo(&name)?.exists() {
            self.verify_repo(&name, &project)?;
        } else {
            self.clone_project(&name, &project)?;
        }
        config.projects.insert(name.clone(), project);
        config.save(&child(&self.0, MANIFEST)?)?;
        println!("Registered {name}");
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
        }
        for name in names {
            let project = &config.projects[&name];
            if self.repo(&name)?.exists() {
                let repo = self.verify_repo(&name, project)?;
                git(&repo, &["fetch", "origin"])?;
            } else {
                self.clone_project(&name, project)?;
            }
            println!("Synced {name}");
        }
        Ok(())
    }

    pub fn task_dir(&self, name: &str) -> Result<PathBuf> {
        name_ok(name)?;
        child(&child(&self.0, "worktrees")?, name)
    }

    pub fn records(&self) -> Result<PathBuf> {
        child(&child(&self.0, ".wk")?, "tasks")
    }

    pub fn record(&self, name: &str) -> Result<PathBuf> {
        name_ok(name)?;
        child(&self.records()?, &format!("{name}.json"))
    }
}
