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
- Creates desktop and start menu shortcuts when `RendogLauncher.exe` is available.
- Registers a Windows uninstaller entry under HKLM.
- Re-runs the uninstaller with administrator permission when needed.
- Skips launch and shortcut creation gracefully while `RendogLauncher.exe` is pending.
- Applies a 60 second timeout to online component downloads.

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
  -> request administrator permission at install start
  -> background install thread
  -> run_install()
  -> download_ready_components()
  -> verify component size and SHA-256
  -> install_downloaded_components()
  -> create shortcuts when launcher artifact exists
  -> register Windows uninstaller
  -> InstallEvent::Progress updates Slint UI
  -> InstallEvent::Completed opens the complete screen
  -> launch installed launcher when checked and available
```

## Uninstall Flow

```text
Windows Apps uninstall entry
  -> RendogLauncherInstaller.exe --uninstall --install-dir <path>
  -> request administrator permission when needed
  -> remove desktop/start menu shortcuts
  -> preserve user-data by moving it next to the install directory
  -> remove HKLM uninstall entry
  -> schedule install directory deletion
```

## Build Check

Use Cargo from the Rust toolchain:

```powershell
cargo check
```

If the current shell does not have Cargo in `PATH`, use:

```powershell
& "$env:USERPROFILE\.cargo\bin\cargo.exe" check
```

For a release executable:

```powershell
& "$env:USERPROFILE\.cargo\bin\cargo.exe" build --release
```
