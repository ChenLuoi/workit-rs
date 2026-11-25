use anyhow::{Result, ensure};
use indicatif::ProgressBar;
use inquire::{
    MultiSelect, Select, Text,
    validator::{MinLengthValidator, Validation},
};
use std::{
    io::{self, IsTerminal},
    time::Duration,
};

pub fn interactive() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

pub fn task_name() -> Result<String> {
    ensure!(
        interactive(),
        "specify a task name: wk start <task> <project>... (interactive input requires a terminal)"
    );
    eprintln!(
        "The task name is also the Git branch in each project. Use / to group tasks, for example feature/login."
    );
    let name = Text::new("Task name")
        .with_validator(|value: &str| {
            Ok(match crate::workspace::task_name_ok(value.trim()) {
                Ok(()) => Validation::Valid,
                Err(_) => {
                    Validation::Invalid("Enter a task name with valid /-separated segments".into())
                }
            })
        })
        .prompt()?;
    Ok(name.trim().to_owned())
}

pub fn progress<T>(label: &str, action: impl FnOnce() -> Result<T>) -> Result<T> {
    if !io::stderr().is_terminal() {
        return action();
    }
    let spinner = ProgressBar::new_spinner().with_message(label.to_owned());
    spinner.tick();
    spinner.enable_steady_tick(Duration::from_millis(100));
    let result = action();
    spinner.finish_and_clear();
    let icon = if result.is_ok() { "✓" } else { "✗" };
    eprintln!("{icon} {label}");
    result
}

pub fn projects(config: &crate::workspace::Config, defaults: &[String]) -> Result<Vec<String>> {
    let checked = config
        .projects
        .keys()
        .enumerate()
        .filter_map(|(i, p)| defaults.contains(p).then_some(i))
        .collect::<Vec<_>>();
    let selected = MultiSelect::new("Select projects", config.projects.values().collect())
        .with_default(&checked)
        .with_help_message("↑/↓ to move, Space to toggle, Enter to confirm, Esc to cancel")
        .with_validator(MinLengthValidator::new(1).with_message("Select at least one project"))
        .prompt()?;
    Ok(selected
        .into_iter()
        .map(|project| project.name.clone())
        .collect())
}

pub fn branch(label: &str, branches: Vec<String>) -> Result<String> {
    Ok(Select::new(label, branches)
        .with_help_message(
            "Type a branch prefix to filter; ↑/↓ to move, Enter to select, Esc to cancel",
        )
        .with_scorer(&|prefix, branch, _, _| branch.starts_with(prefix).then_some(0))
        // Keep the caller's preferred branch first, including after clearing a filter.
        .with_sorter(&|_| {})
        .prompt()?)
}
