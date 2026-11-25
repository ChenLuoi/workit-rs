use crate::workspace::{Config, Project, Workspace};
use anyhow::{Context, Result};
use serde::de::{
    Deserialize, DeserializeSeed, Deserializer, Error, IgnoredAny, MapAccess, Visitor,
};
use std::fmt;

pub(super) fn read(ws: &Workspace) -> Result<Config<Result<Project>>> {
    let source = ws.manifest_text()?;
    if let Ok(config) = serde_saphyr::from_str::<Config>(&source) {
        return Ok(Config {
            version: config.version,
            tools: config.tools,
            skills: config.skills,
            presets: config.presets,
            projects: config
                .projects
                .into_iter()
                .map(|(name, p)| (name, Ok(p)))
                .collect(),
        });
    }

    // On a damaged manifest, recover each entry using the same typed YAML decoder
    // as the normal command path. This preserves scalar spellings and YAML aliases.
    let inventory: Config<IgnoredAny> = serde_saphyr::from_str(&source).context("parse wk.yaml")?;
    let projects = inventory
        .projects
        .into_keys()
        .map(|name| {
            let item = serde_saphyr::with_deserializer_from_str(&source, |de| {
                ProjectAt(&["projects", &name]).deserialize(de)
            });
            (name, item.map_err(Into::into))
        })
        .collect();
    Ok(Config {
        version: inventory.version,
        tools: inventory.tools,
        skills: inventory.skills,
        presets: inventory.presets,
        projects,
    })
}

struct ProjectAt<'a>(&'a [&'a str]);

impl<'de> DeserializeSeed<'de> for ProjectAt<'_> {
    type Value = Project;

    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<Project, D::Error> {
        if self.0.is_empty() {
            Project::deserialize(de)
        } else {
            de.deserialize_map(self)
        }
    }
}

impl<'de> Visitor<'de> for ProjectAt<'_> {
    type Value = Project;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a project mapping")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Project, A::Error> {
        let (key, rest) = self.0.split_first().expect("nonempty path for a mapping");
        let mut project = None;
        while let Some(field) = map.next_key::<String>()? {
            if field == *key {
                project = Some(map.next_value_seed(ProjectAt(rest))?);
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        project.ok_or_else(|| A::Error::custom(format!("missing key {key}")))
    }
}
