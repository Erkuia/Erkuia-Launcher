# Rendog Launcher Installer

Custom online installer for Rendog Launcher.

## Identity

- Package name: `rendog-launcher-installer`
- Product installed by this app: `Rendog Launcher`
- Target binary name: `RendogLauncherInstaller.exe`
- Install mode: online only
- UI stack: Rust + Slint

## Language and Structure

- Rust owns installer state, file IO, download, verification, and Windows integration.
- Slint owns the native installer UI and Figma-based screen layout.
- `manifest.json` defines installable components and progress stage weights.
- `src/progress.rs` defines the event model used to update the UI in real time.
- `src/install.rs` connects prepare, download, verification, file installation, and finalization.

## Installer Behavior

- Requires administrator permission when installation starts.
- Uses `installer/manifest.json` as the installation manifest.
- Downloads only components marked as `ready`.
- Leaves pending components out of the download flow.
- Shows real-time progress from actual download, verification, and file copy work.
- Defaults `run after install` to checked.
- Defaults `create desktop shortcut` to checked.
- Includes uninstaller support as a planned installer feature.

## Current Install Components

### Ready

- `RendogClient-Delta.jar`
- Source: `https://github.com/MellDa1024/RendogClient-1.20.4/releases/download/Delta/RendogClient-Delta.jar`
- Target path: `minecraft/mods/RendogClient-Delta.jar`
- SHA-256: `72fc258a685734e9cb7914aca0cabf60696facb2253b48dd959eede94b1c111a`

### Pending

- `RendogLauncher.exe`
- Reason: launcher artifact is intentionally deferred.

## Progress Stages

- Prepare: `0-5%`
- Download: `5-50%`
- Verify: `50-65%`
- Install files: `65-88%`
- Shortcuts: `88-94%`
- Register uninstaller: `94-98%`
- Finalize: `98-100%`

## Installer Flow

```text
Start screen
  -> select install path
  -> start install with administrator permission
  -> prepare install directory and cache
  -> download ready components
  -> verify downloaded files
  -> copy files to target paths
  -> create desktop/start menu shortcuts
  -> register uninstaller
  -> complete screen
  -> run RendogLauncher.exe when checked
```

## Current Implementation Flow

```text
UI install button
  -> background install thread
  -> run_install()
  -> download_ready_components()
  -> install_downloaded_components()
  -> InstallEvent::Progress updates Slint UI
  -> InstallEvent::Completed opens the complete screen
```

Shortcut creation, uninstaller registration, and launching `RendogLauncher.exe` are planned next-step integrations.

## Build Check

Use Cargo from the Rust toolchain:

```powershell
cargo check
```

If the current shell does not have Cargo in `PATH`, use:

```powershell
& "$env:USERPROFILE\.cargo\bin\cargo.exe" check
```
