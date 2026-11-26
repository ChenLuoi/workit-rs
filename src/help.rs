use clap::Command;
use std::fmt::Write;

const GROUPS: &[(&str, &[&str])] = &[
    ("Daily tasks", &["start", "run", "status", "path", "close"]),
    ("Workspace", &["init", "setup", "sync", "doctor"]),
    ("Projects and presets", &["add", "remove", "preset"]),
];

pub(super) fn group_root_commands(root: Command) -> Command {
    let mut sections = String::new();
    for &(heading, names) in GROUPS {
        let commands = names.iter().enumerate().map(|(order, &name)| {
            root.find_subcommand(name)
                .unwrap_or_else(|| panic!("unknown command in help group: {name}"))
                .clone()
                .display_order(order)
        });
        // Let clap lay out each section using the actual commands, summaries and aliases.
        let mut section = Command::new("wk")
            .subcommands(commands)
            .subcommand_help_heading(heading)
            .disable_help_flag(true)
            .disable_help_subcommand(true)
            .help_template("{all-args}");
        writeln!(sections, "{}", section.render_help().ansi()).unwrap();
    }

    let header = root.get_styles().get_header();
    let template = format!(
        "{{about-with-newline}}\n{{usage-heading}} {{usage}}\n\n{{before-help}}{header}Options:{header:#}\n{{options}}\n\nRun 'wk help <command>' for details."
    );
    root.before_help(sections.trim_end().to_owned())
        .help_template(template)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Cli;
    use clap::CommandFactory;

    #[test]
    fn each_visible_command_is_grouped_exactly_once() {
        let root = Cli::command();
        let mut visible: Vec<_> = root
            .get_subcommands()
            .filter(|c| !c.is_hide_set())
            .map(Command::get_name)
            .collect();
        let mut grouped: Vec<_> = GROUPS
            .iter()
            .flat_map(|(_, names)| names.iter().copied())
            .collect();
        visible.sort_unstable();
        grouped.sort_unstable();
        assert_eq!(grouped, visible);
    }
}
