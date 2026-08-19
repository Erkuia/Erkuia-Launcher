use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

use anyhow::Context;

pub fn command() -> Command {
    Command::new(executable_path())
}

pub fn output(args: &[&str]) -> anyhow::Result<Output> {
    command()
        .args(args)
        .output()
        .context("failed to run PowerShell")
}

pub fn spawn_hidden(script: &str) -> anyhow::Result<()> {
    command()
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", script])
        .spawn()
        .context("failed to spawn hidden PowerShell")?;

    Ok(())
}

pub fn escape_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
}

pub fn escape_double_quoted(value: &str) -> String {
    value.replace('`', "``").replace('"', "`\"")
}

fn executable_path() -> PathBuf {
    let system_root = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let powershell = system_root
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");

    if Path::new(&powershell).exists() {
        return powershell;
    }

    PathBuf::from("powershell.exe")
}
