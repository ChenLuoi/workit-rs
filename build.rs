use std::{env, fmt::Write, fs, io, path::Path, path::PathBuf};

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            collect_files(root, &path, files)?;
        } else if kind.is_file() {
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .expect("UTF-8 template path");
            files.push(relative.replace(std::path::MAIN_SEPARATOR, "/"));
        } else {
            return Err(io::Error::other(format!(
                "expected a regular template file at {}",
                path.display()
            )));
        }
    }
    Ok(())
}

fn main() -> io::Result<()> {
    println!("cargo:rerun-if-changed=templates");
    let root = Path::new("templates/workspace");
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();
    let mut table = String::from("&[\n");
    for name in files {
        writeln!(table, "({name:?}, include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/templates/workspace/\", {name:?}))),").unwrap();
    }
    table.push_str("]\n");
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("workspace_files.rs"),
        table,
    )
}
