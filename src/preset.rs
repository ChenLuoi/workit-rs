use crate::{
    prompt,
    workspace::{Config, MANIFEST, Workspace, child, name_ok, save_yaml, validate_base},
};
use anyhow::{Context, Result, ensure};
use clap::Subcommand;
use inquire::{Select, Text};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectBase {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
}

#[derive(Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub projects: BTreeMap<String, ProjectBase>,
    #[serde(default)]
    pub skills: BTreeSet<String>,
}

impl Preset {
    pub fn base<'a>(&'a self, config: &'a Config, project: &str) -> &'a str {
        self.projects
            .get(project)
            .and_then(|p| p.base.as_deref())
            .unwrap_or(&config.projects[project].base)
    }

    pub fn check<P>(&self, ws: &Workspace, config: &Config<P>) -> Result<()> {
        ensure!(
            !self.projects.is_empty(),
            "select at least one registered project"
        );
        for (name, entry) in &self.projects {
            ensure!(config.projects.contains_key(name), "unknown project {name}");
            if let Some(base) = &entry.base {
                validate_base(base).with_context(|| format!("project {name}"))?;
            }
        }
        for id in &self.skills {
            ws.task_skill(id)
                .with_context(|| format!("task skill {id}"))?;
        }
        Ok(())
    }
}

#[derive(Subcommand)]
pub enum PresetAction {
    /// Create a preset interactively.
    Add { name: String },
    /// Edit projects, base overrides and skills interactively.
    Edit { name: String },
    /// List saved workspace presets.
    List,
    /// Display the saved configuration for one preset.
    Show { name: String },
    /// Delete a preset; existing tasks keep their recorded selections.
    #[command(visible_alias = "remove")]
    Rm { name: String },
}

fn label(name: &str, preset: &Preset) -> String {
    match &preset.description {
        Some(description) => format!("{name} — {description}"),
        None => name.to_owned(),
    }
}

pub fn choose(config: &Config) -> Result<Option<String>> {
    let mut choices = vec!["Start from scratch".to_owned()];
    choices.extend(config.presets.iter().map(|(name, p)| label(name, p)));
    let choice = Select::new("Task preset", choices).raw_prompt()?;
    Ok(choice
        .index
        .checked_sub(1)
        .and_then(|i| config.presets.keys().nth(i))
        .cloned())
}

impl Workspace {
    pub fn preset(&self, action: PresetAction) -> Result<()> {
        let mut config = self.config()?;
        match action {
            PresetAction::List => {
                if config.presets.is_empty() {
                    println!("No presets. Create one with wk preset add <name>.");
                }
                for (name, p) in &config.presets {
                    println!("{}", label(name, p));
                }
            }
            PresetAction::Show { name } => {
                let preset = config
                    .presets
                    .get(&name)
                    .with_context(|| format!("unknown preset {name}"))?;
                print!("{}", serde_saphyr::to_string(preset)?);
            }
            PresetAction::Rm { name } => {
                let _lock = self.lock()?;
                let mut config = self.config()?;
                config
                    .presets
                    .remove(&name)
                    .with_context(|| format!("unknown preset {name}"))?;
                save_yaml(&child(&self.0, MANIFEST)?, &config)?;
                println!("Removed preset {name}");
            }
            PresetAction::Add { ref name } | PresetAction::Edit { ref name } => {
                name_ok(name)?;
                let old = config.presets.remove(name);
                if matches!(action, PresetAction::Add { .. }) {
                    ensure!(old.is_none(), "preset {name} already exists");
                } else {
                    ensure!(old.is_some(), "unknown preset {name}");
                }
                ensure!(
                    !config.projects.is_empty(),
                    "register a project first with wk add <url>"
                );
                ensure!(prompt::interactive(), "preset setup requires a terminal");
                let mut result = old.clone().unwrap_or_default();
                let desc = Text::new("Preset description")
                    .with_initial_value(result.description.as_deref().unwrap_or(""))
                    .prompt()?;
                result.description = Some(desc.trim().to_owned()).filter(|s| !s.is_empty());
                let chosen = prompt::projects(
                    config.projects.values().collect(),
                    &result.projects.keys().cloned().collect::<Vec<_>>(),
                    false,
                )?;
                let mut projects = BTreeMap::new();
                for p in chosen {
                    let current = result
                        .projects
                        .get(&p)
                        .and_then(|entry| entry.base.as_deref())
                        .unwrap_or("");
                    let help = format!(
                        "Leave empty to inherit the project's base ({})",
                        config.projects[&p].base
                    );
                    let base = Text::new(&format!("Base override for {p}"))
                        .with_initial_value(current)
                        .with_help_message(&help)
                        .prompt()?;
                    let base = Some(base.trim().to_owned()).filter(|v| !v.is_empty());
                    projects.insert(p, ProjectBase { base });
                }
                result.projects = projects;
                result.skills = self.choose_task_skills(&result.skills)?;
                let _lock = self.lock()?;
                let mut latest = self.config()?;
                ensure!(
                    latest.presets.get(name) == old.as_ref(),
                    "preset {name} changed during setup; retry"
                );
                result.check(self, &latest)?;
                latest.presets.insert(name.clone(), result);
                save_yaml(&child(&self.0, MANIFEST)?, &latest)?;
                println!("Saved preset {name}");
            }
        }
        Ok(())
    }
}
