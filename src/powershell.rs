use std::{
    path::{Path, PathBuf},
    process::{Command, Output},
};

use anyhow::Context;

/// Windows `CREATE_NO_WINDOW`. Without it every PowerShell helper spawned from
/// this GUI-subsystem binary flashes a console window on screen.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn command() -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(executable_path());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    command
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

/// Quote a single value the way the Windows command line parser expects, so it
/// survives being handed to `Start-Process -ArgumentList` as one argument even
/// when it contains spaces or trailing backslashes.
pub fn quote_command_line_arg(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');

    let mut backslashes = 0_usize;
    for character in value.chars() {
        match character {
            '\\' => {
                backslashes += 1;
                quoted.push(character);
            }
            '"' => {
                for _ in 0..=backslashes {
                    quoted.push('\\');
                }
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                backslashes = 0;
                quoted.push(character);
            }
        }
    }

    for _ in 0..backslashes {
        quoted.push('\\');
    }
    quoted.push('"');

    quoted
}

pub fn join_command_line_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_command_line_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
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
