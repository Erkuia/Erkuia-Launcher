# [Erkuia Launcher](docs.md)

Lightweight customized Minecraft launcher for the Erkuia server.

## Product Direction

Erkuia Launcher is a dedicated launcher for one server and one Minecraft version. It is not intended to behave like a general-purpose Minecraft launcher.

## Fixed Server Target

- Publisher / company name: `Erkuia`
- Minecraft version: `1.21.4`
- Server address: `erkuia.kr`
- Launcher product name: `Erkuia Launcher`
- Launcher binary name: `Erkuia-Launcher.exe`

## Runtime Direction

- Launcher language: Rust
- Minecraft client mod language: Java 21
- Rendering optimization: Java 21 + GLSL
- Launcher should close after starting Minecraft to avoid idle memory usage.
- Minecraft should auto-connect to the Erkuia server.
- Leaving the server should close Minecraft and return control to the launcher flow.

## Language and Responsibilities

| Area | Language | Responsibility |
|---|---|---|
| Launcher app | Rust | UI, auth/session, updater, file verification, mod profile control |
| JVM launcher | Rust | Java runtime detection, JVM argument generation, Minecraft process start |
| Client mod | Java 21 | Fabric/Mixin hooks, server auto-connect, disconnect handling |

## Launcher Flow

```text
Open Erkuia-Launcher.exe
  -> check login/session
  -> check launcher/client manifest
  -> download or repair changed files
  -> apply selected mod ON/OFF state
  -> prepare bundled Java runtime
  -> build JVM launch arguments
  -> start Minecraft 1.21.4
  -> close launcher process
  -> Minecraft auto-connects to erkuia.kr
  -> server disconnect closes Minecraft
  -> launcher flow can be started again
```

## Planned Launcher Features

- Fixed Minecraft/Fabric version.
- Mod ON/OFF management from the launcher.
- Online update and file verification.
- Dedicated Java runtime management.
- JVM launch configuration.
- Server auto-connect support through the launcher-owned client mod.
- Memory-focused launch behavior.

## Optimization Goals

- Remove unnecessary launcher-side UI and profile complexity.
- Avoid keeping the launcher process resident while the game is running.
- Prefer loading control and runtime policy over unsafe deletion of Minecraft assets.
