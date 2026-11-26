use crate::{
    prompt, templates,
    tool::{Tool, pick_tools, select_tools},
    workspace::{Config, MANIFEST, Workspace, child, name_ok, project_description, save_yaml},
};
use anyhow::{Context, Result, ensure};
use clap::Args;
use inquire::{MultiSelect, Select};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, io::ErrorKind, path::PathBuf};

#[derive(Args)]
pub struct SetupOptions {
    /// Configure this checkout's projects and tools in .wk/local.yaml.
    #[arg(long, conflicts_with = "shared")]
    local: bool,
    /// Configure team settings in wk.yaml.
    #[arg(long)]
    shared: bool,
    /// Use exactly these registered projects locally.
    #[arg(value_name = "PROJECT", conflicts_with_all = ["shared", "all_projects"])]
    projects: Vec<String>,
    /// Follow all current and future registered projects locally.
    #[arg(long, conflicts_with = "shared")]
    all_projects: bool,
    /// Select tools for this scope (comma-separated). Pass --tools alone to clear.
    #[arg(long, value_delimiter = ',', num_args = 0..=1)]
    tools: Option<Vec<Tool>>,
    /// Inherit the shared tool selection locally.
    #[arg(long, conflicts_with_all = ["shared", "tools"])]
    all_tools: bool,
    /// Register these bundled workspace skills. Pass --skills alone to clear.
    #[arg(long, value_delimiter = ',', num_args = 0..=1, conflicts_with = "local",
        value_parser = clap::builder::PossibleValuesParser::new(templates::skill_names()))]
    skills: Option<Vec<String>>,
    /// Edit this project's shared description.
    #[arg(long, conflicts_with = "local")]
    project: Option<String>,
    /// Set the project description (an empty string clears it).
    #[arg(long, requires = "project")]
    description: Option<String>,
}

#[derive(Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    projects: Option<BTreeSet<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<BTreeSet<Tool>>,
}

impl LocalConfig {
    pub fn includes(&self, name: &str) -> bool {
        self.projects
            .as_ref()
            .is_none_or(|names| names.contains(name))
    }

    pub fn enabled_tools(&self, config: &Config) -> BTreeSet<Tool> {
        match &self.tools {
            Some(tools) => tools.intersection(&config.tools).copied().collect(),
            None => config.tools.clone(),
        }
    }

    pub fn require_projects<'a>(&self, names: impl IntoIterator<Item = &'a String>) -> Result<()> {
        let disabled: Vec<_> = names
            .into_iter()
            .filter(|p| !self.includes(p))
            .map(String::as_str)
            .collect();
        ensure!(
            disabled.is_empty(),
            "disabled locally: {}. Enable with wk setup --local",
            disabled.join(", ")
        );
        Ok(())
    }
}

// An explicit selection stays fixed even when it currently contains every choice.
fn follow_shared(label: &str, following: bool) -> Result<bool> {
    Ok(Select::new(
        label,
        vec!["Follow all shared choices", "Choose a fixed selection"],
    )
    .with_starting_cursor(usize::from(!following))
    .raw_prompt()?
    .index
        == 0)
}

impl Workspace {
    fn local_file(&self) -> Result<PathBuf> {
        child(&child(&self.0, ".wk")?, "local.yaml")
    }

    fn read_local_config(&self) -> Result<Option<LocalConfig>> {
        let text = match fs::read_to_string(self.local_file()?) {
            Ok(text) => text,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).context("read .wk/local.yaml"),
        };
        let local: LocalConfig = serde_saphyr::from_str(&text).context("parse .wk/local.yaml")?;
        if let Some(names) = &local.projects {
            for p in names {
                name_ok(p).context("invalid project name in .wk/local.yaml")?;
            }
        }
        Ok(Some(local))
    }

    pub(crate) fn local_config(&self) -> Result<LocalConfig> {
        Ok(self.read_local_config()?.unwrap_or_default())
    }

    pub fn setup(&self, options: SetupOptions) -> Result<()> {
        let shared = if options.local || options.shared {
            options.shared
        } else {
            ensure!(
                prompt::interactive(),
                "choose a configuration scope with --local or --shared outside a terminal"
            );
            Select::new(
                "Configuration scope",
                vec![
                    "Personal settings (.wk/local.yaml)",
                    "Shared settings (wk.yaml)",
                ],
            )
            .raw_prompt()?
            .index
                == 1
        };
        let config = self.config()?;
        if shared {
            self.setup_shared(config, options)
        } else {
            self.setup_local(config, options)
        }
    }

    fn setup_local(&self, config: Config, options: SetupOptions) -> Result<()> {
        ensure!(
            options.project.is_none() && options.skills.is_none(),
            "project descriptions and workspace skills require shared setup"
        );
        let previous = self.read_local_config()?;
        let old = previous.clone().unwrap_or_default();
        let mut local = old.clone();
        let mut edit_projects = options.all_projects || !options.projects.is_empty();
        if edit_projects {
            for p in &options.projects {
                ensure!(config.projects.contains_key(p), "unknown project {p}");
            }
            local.projects =
                (!options.all_projects).then(|| options.projects.into_iter().collect());
        }
        if let Some(tools) = options.tools {
            for t in &tools {
                ensure!(
                    config.tools.contains(t),
                    "{t} is not enabled in shared settings"
                );
            }
            local.tools = Some(tools.into_iter().collect());
        } else if options.all_tools {
            local.tools = None;
        } else if !edit_projects {
            ensure!(
                prompt::interactive(),
                "supply project names, --all-projects, --tools or --all-tools outside a terminal"
            );
            let defaults: &[usize] = if previous.is_none() { &[0, 1] } else { &[] };
            let fields = MultiSelect::new("Personal settings to edit", vec!["Projects", "Tools"])
                .with_default(defaults)
                .prompt()?;
            if fields.contains(&"Projects") {
                edit_projects = true;
                local.projects = if follow_shared("Local projects", local.projects.is_none())? {
                    None
                } else if config.projects.is_empty() {
                    Some(BTreeSet::new())
                } else {
                    let checked = config
                        .projects
                        .keys()
                        .filter(|p| old.includes(p))
                        .cloned()
                        .collect::<Vec<_>>();
                    Some(
                        prompt::projects(config.projects.values().collect(), &checked, true)?
                            .into_iter()
                            .collect(),
                    )
                };
            }
            if fields.contains(&"Tools") {
                local.tools = if follow_shared("Local tools", local.tools.is_none())? {
                    None
                } else {
                    Some(pick_tools(
                        &config.tools.iter().copied().collect::<Vec<_>>(),
                        Some(&old.enabled_tools(&config)),
                    )?)
                };
            }
        }
        let _lock = self.lock()?;
        ensure!(
            self.config()? == config && self.read_local_config()? == previous,
            "settings changed during setup; retry"
        );
        for dir in ["repos", "tasks"] {
            fs::create_dir_all(child(&self.0, dir)?)?;
        }
        self.ensure_active_index()?;
        if previous.as_ref() != Some(&local) {
            save_yaml(&self.local_file()?, &local)?;
        }
        println!("Personal settings saved in .wk/local.yaml");
        if edit_projects {
            let mut names = Vec::new();
            let mut skipped = 0;
            for name in config.projects.keys() {
                if !local.includes(name) {
                    skipped += 1;
                } else if previous.is_none()
                    || !old.includes(name)
                    // Let sync report an inaccessible destination alongside other project failures.
                    || !self.repo(name).and_then(|repo| Ok(repo.try_exists()?)).is_ok_and(|exists| exists)
                {
                    names.push(name.clone());
                }
            }
            self.sync_projects(&config, &names, skipped)?;
        }
        Ok(())
    }

    fn setup_shared(&self, original: Config, mut options: SetupOptions) -> Result<()> {
        ensure!(
            options.projects.is_empty() && !options.all_projects && !options.all_tools,
            "personal project and inheritance settings require local setup"
        );
        let mut config = original.clone();
        let wizard =
            options.tools.is_none() && options.skills.is_none() && options.project.is_none();
        let mut edit_skills = options.skills.is_some();
        if wizard {
            ensure!(
                prompt::interactive(),
                "supply --tools, --skills or --project and --description outside a terminal"
            );
            let mut choices = vec!["Tools", "Workspace skills"];
            if !config.projects.is_empty() {
                choices.push("Project description");
            }
            let fields = MultiSelect::new("Shared settings to edit", choices).prompt()?;
            if fields.contains(&"Tools") {
                config.tools = select_tools(Some(&config.tools))?;
            }
            if fields.contains(&"Workspace skills") {
                edit_skills = true;
                config.skills = templates::select_skills(Some(&config.skills))?;
            }
            if fields.contains(&"Project description") {
                options.project = Some(
                    Select::new(
                        "Project to describe",
                        config.projects.keys().cloned().collect(),
                    )
                    .prompt()?,
                );
            }
        }
        if let Some(tools) = options.tools {
            config.tools = tools.into_iter().collect();
        }
        if let Some(skills) = options.skills {
            config.skills = skills.into_iter().collect();
        }
        if let Some(name) = options.project {
            let project = config
                .projects
                .get_mut(&name)
                .with_context(|| format!("unknown project {name}"))?;
            ensure!(
                options.description.is_some() || prompt::interactive(),
                "supply --description outside a terminal"
            );
            project.description = project_description(
                options.description,
                true,
                project.description.as_deref().unwrap_or(""),
            )?;
        }
        let _lock = self.lock()?;
        ensure!(
            self.config()? == original,
            "shared settings changed during setup; retry"
        );
        if edit_skills {
            templates::install_skills(self, &config.skills)?;
        }
        if config != original {
            save_yaml(&child(&self.0, MANIFEST)?, &config)?;
        }
        println!("Shared settings saved in wk.yaml");
        Ok(())
    }
}
