# Rendog Launcher

Lightweight customized Minecraft launcher for RendogServer.

## Product Direction

Rendog Launcher is a dedicated launcher for one server and one Minecraft version. It is not intended to behave like a general-purpose Minecraft launcher.

## Fixed Server Target

- Minecraft version: `1.20.4`
- Server address: `rendog.kr`
- Client mod source: [RendogClient-1.20.4](https://github.com/MellDa1024/RendogClient-1.20.4)

## Runtime Direction

- Launcher language: Rust
- Minecraft client mod language: Java 21
- Rendering optimization: Java 21 + GLSL
- Launcher should close after starting Minecraft to avoid idle memory usage.
- Minecraft should auto-connect to the Rendog server.
- Leaving the server should close Minecraft and return control to the launcher flow.

## Language and Responsibilities

| Area | Language | Responsibility |
|---|---|---|
| Launcher app | Rust | UI, auth/session, updater, file verification, mod profile control |
| JVM launcher | Rust | Java runtime detection, JVM argument generation, Minecraft process start |
| Client mod | Java 21 | Fabric/Mixin hooks, server auto-connect, disconnect handling |
| Adaptive renderer | Java 21 + GLSL | FPS target policy, LOD/mipmap policy, shader-level quality scaling |

## Launcher Flow

```text
Open RendogLauncher.exe
  -> check login/session
  -> check launcher/client manifest
  -> download or repair changed files
  -> apply selected mod ON/OFF state
  -> prepare bundled Java runtime
  -> build JVM launch arguments
  -> start Minecraft 1.20.4
  -> close launcher process
  -> Minecraft auto-connects to rendog.kr
  -> server disconnect closes Minecraft
  -> launcher flow can be started again
```

## Planned Launcher Features

- Fixed Minecraft/Fabric version.
- Mod ON/OFF management from the launcher.
- Online update and file verification.
- Dedicated Java runtime management.
- JVM launch configuration.
- Server auto-connect support through the Rendog client mod.
- Memory-focused launch behavior.

## Optimization Goals

- Remove unnecessary launcher-side UI and profile complexity.
- Avoid keeping the launcher process resident while the game is running.
- Support adaptive FPS behavior through the client mod.
- Prefer loading control and runtime policy over unsafe deletion of Minecraft assets.
