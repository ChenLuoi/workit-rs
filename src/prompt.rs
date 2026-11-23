use anyhow::Result;
use inquire::{MultiSelect, Select, validator::MinLengthValidator};
use std::io::{self, IsTerminal};

pub fn interactive() -> bool {
    io::stdin().is_terminal() && io::stderr().is_terminal()
}

pub fn projects(names: Vec<String>) -> Result<Vec<String>> {
    Ok(MultiSelect::new("Select projects", names)
        .with_help_message("↑/↓ to move, Space to toggle, Enter to confirm, Esc to cancel")
        .with_validator(MinLengthValidator::new(1).with_message("Select at least one project"))
        .prompt()?)
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
