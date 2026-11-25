mod skills;
pub use skills::{skill_differences, sync_skills};

use crate::{tasks::Task, workspace::child};
use anyhow::{Context, Result, ensure};
use inquire::MultiSelect;
use serde::{Deserialize, Deserializer, de::Error};
use std::{collections::BTreeSet, fmt, fs, io::Write, path::Path};
use tempfile::NamedTempFile;

const WORKSPACE_FILES: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/workspace_files.rs"));
const SKILL_DIR: &str = ".agents/skills/";

fn skill_file(path: &str) -> Option<(&str, &str)> {
    path.strip_prefix(SKILL_DIR)?.split_once('/')
}

fn skill_files() -> impl Iterator<Item = (&'static str, &'static [u8])> {
    WORKSPACE_FILES
        .iter()
        .filter_map(|&(path, bytes)| match skill_file(path) {
            Some((name, "SKILL.md")) => Some((name, bytes)),
            _ => None,
        })
}

pub fn skill_names() -> impl Iterator<Item = &'static str> {
    skill_files().map(|(name, _)| name)
}

#[derive(Deserialize)]
pub(crate) struct Skill {
    pub name: String,
    description: String,
}

impl Skill {
    pub fn read(id: &str, bytes: &[u8]) -> Result<Self> {
        let text = std::str::from_utf8(bytes)?.replace("\r\n", "\n");
        let (header, body) = text
            .strip_prefix("---\n")
            .and_then(|s| s.split_once("\n---\n"))
            .context("missing YAML frontmatter")?;
        let skill: Self = serde_saphyr::from_str(header)?;
        ensure!(skill.name == id, "skill name must match its directory");
        ensure!(
            !skill.description.trim().is_empty() && !body.trim().is_empty(),
            "missing skill description or instructions"
        );
        Ok(skill)
    }
}

impl fmt::Display for Skill {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}\n    {}", self.name, self.description.trim())
    }
}

fn skill_choices() -> Result<Vec<Skill>> {
    skill_files()
        .map(|(id, bytes)| {
            Skill::read(id, bytes).with_context(|| format!("read {SKILL_DIR}{id}/SKILL.md"))
        })
        .collect()
}

pub fn deserialize_skills<'de, D: Deserializer<'de>>(de: D) -> Result<BTreeSet<String>, D::Error> {
    let names = BTreeSet::<String>::deserialize(de)?;
    for name in &names {
        if !skill_names().any(|known| known == name) {
            return Err(D::Error::custom(format!("unknown bundled skill {name}")));
        }
    }
    Ok(names)
}

pub fn select_skills() -> Result<BTreeSet<String>> {
    eprintln!("Choose optional skills to copy into .agents/skills/ for this workspace.");
    let choices = skill_choices()?;
    let defaults = (0..choices.len()).collect::<Vec<_>>();
    pick_skills("Install skills", choices, &defaults)
}

pub fn pick_skills(
    label: &str,
    choices: Vec<Skill>,
    defaults: &[usize],
) -> Result<BTreeSet<String>> {
    let selected = MultiSelect::new(label, choices)
        .with_default(defaults)
        .with_help_message("Space toggles a skill, Enter confirms. Leave all unchecked to skip.")
        .with_formatter(&|answers| {
            answers
                .iter()
                .map(|a| a.value.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .prompt()?;
    Ok(selected.into_iter().map(|skill| skill.name).collect())
}

pub fn install(root: &Path, skills: &BTreeSet<String>) -> Result<()> {
    for &(relative, contents) in WORKSPACE_FILES {
        if let Some((name, _)) = skill_file(relative)
            && !skills.contains(name)
        {
            continue;
        }
        install_missing(root, relative, contents)?;
    }
    Ok(())
}

fn install_missing(root: &Path, relative: &str, contents: &[u8]) -> Result<()> {
    let mut parent = root.to_path_buf();
    let mut parts = relative.split('/');
    let filename = parts.next_back().unwrap();
    for part in parts {
        parent = child(&parent, part)?;
        fs::create_dir_all(&parent)?;
    }
    let path = child(&parent, filename)?;
    if path.try_exists()? {
        ensure!(path.is_file(), "expected a file at {}", path.display());
    } else {
        let mut tmp = NamedTempFile::new_in(&parent)?;
        tmp.write_all(contents)?;
        tmp.persist_noclobber(path)?;
    }
    Ok(())
}

pub fn install_task_docs(dir: &Path, name: &str, task: &Task) -> Result<()> {
    let mut skills = String::new();
    if !task.skills.is_empty() {
        skills.push_str("## Task skills\n\n");
        for id in &task.skills {
            skills.push_str(&format!("- [{id}](.agents/skills/{id}/SKILL.md)\n"));
        }
        skills.push('\n');
    }
    let projects = task
        .projects
        .iter()
        .map(|(name, member)| {
            let mut item = format!("- `{name}/`");
            if let Some(role) = &member.description {
                item.push_str(" — ");
                item.push_str(role);
            }
            format!(
                "{item}\n  - Starting ref and commit: `{}` at `{}`",
                member.base, member.commit
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    for (file, template) in [
        ("README.md", include_str!("../templates/task/README.md")),
        ("AGENTS.md", include_str!("../templates/task/AGENTS.md")),
    ] {
        let doc = template
            .replace("\r\n", "\n")
            .replace("{{task}}", name)
            .replace("{{skills}}\n", &skills)
            .replace("{{projects}}", &projects);
        install_missing(dir, file, doc.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_skill_metadata_matches_the_catalog() {
        assert!(
            skill_choices()
                .unwrap()
                .iter()
                .map(|s| s.name.as_str())
                .eq(skill_names())
        );
    }

    #[test]
    fn picker_displays_the_yaml_description() {
        let skill = Skill::read("demo", b"---\r\nname: demo\r\ndescription: >-\r\n  Compare snapshots\r\n  across projects.\r\n---\r\nInstructions.").unwrap();
        assert_eq!(
            skill.to_string(),
            "demo\n    Compare snapshots across projects."
        );
    }
}
