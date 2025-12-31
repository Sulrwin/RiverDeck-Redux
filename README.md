# RiverDeck-Redux

**RiverDeck-Redux** is a clean-room Rust implementation of Stream Deck controller software, built with **Iced** and targeting **Linux + Windows**.

## Project status

This repository is under active development, but the core MVP layers are already implemented:

- **Workspace bootstrap**: Rust workspace, CI (fmt/clippy/test), licensing, crate boundaries
- **Device MVP**: HID device discovery/connect, key events, brightness, basic key image push
- **UI MVP (Iced)**: device picker, live key grid, brightness control
- **Profiles MVP**: versioned JSON profiles on disk, migrations, minimal per-key label editor
- **OpenAction MVP (local)**: install plugins from a local directory, parse `manifest.json`, bind actions to keys, basic schema-driven settings UI, invoke plugin executable on key-down

## Current features (implemented)

- **GUI (Iced)**:
  - device discovery + connect
  - live key grid that highlights key presses
  - brightness slider (sends to device)
  - profiles: create/select/edit key labels + save to disk
  - plugins: local install + list installed + bind action + edit action settings
- **CLI tools** (for bring-up and debugging):
  - list devices, watch events, set brightness, push a solid-color test image
- **Storage**:
  - profiles stored as JSON with schema versioning (currently v2; v1 auto-migrates on load)
  - plugin installs copied into the app data directory

## Planned features (next)

- **OpenAction**:
  - marketplace install/auth flows (beyond local install)
  - richer settings schema + more field types/validation
  - better plugin process lifecycle (long-running plugins, bidirectional IPC, logs)
- **Profiles / UX**:
  - richer per-key appearance (text layout, images, icon packs)
  - multi-actions / toggles / timers
  - per-app profile switching (platform-specific window detection)
- **Device support & reliability**:
  - improved Stream Deck model coverage and protocol hardening
  - faster/safer image pipeline (resize/dither/caching)
  - reconnect handling and better error surfaces in UI
- **Packaging**:
  - Windows installer and Linux packaging
  - macOS support after MVP

## Architecture (at a glance)

- `crates/ui-iced/`: Iced application (UI + async command wiring)
- `crates/device/`: device service abstraction and Stream Deck implementation
- `crates/transport-hid/`: `hidapi` wrapper for Linux/Windows HID transport
- `crates/render/`: rendering helpers (currently includes test patterns)
- `crates/storage/`: paths + profile persistence/migrations
- `crates/openaction/`: OpenAction manifest model + local plugin registry/installer
- `crates/plugin-runtime/`: plugin action invocation (spawns plugin process)
- `crates/cli/`: bring-up CLI utilities

## Build & run

Prerequisites: a recent Rust toolchain.

Run the app (Iced UI):

```bash
cargo run -p ui-iced
```

## CLI usage (hardware bring-up)

```bash
cargo run -p cli -- help
cargo run -p cli -- list
cargo run -p cli -- events <device_id>
cargo run -p cli -- brightness <device_id> <percent>
cargo run -p cli -- test-image <device_id> <key> <r> <g> <b>
```

## Profiles

- Profiles are stored as JSON on disk (schema versioned).
- The UI currently lets you:
  - create/select profiles
  - edit per-key label
  - bind an action (from installed plugins) to a key
  - edit action settings and save

## OpenAction (local plugin MVP)

### Installing a plugin (local directory)

In the UI, paste a local plugin directory path that contains `manifest.json` and click **Install**.
The directory is copied into the app data directory under `plugins/<plugin_id>/`.

### Minimal `manifest.json` shape (current MVP)

This project currently expects a minimal manifest model:

- `id`: unique plugin ID (used as install directory name)
- `name`: plugin display name
- `actions`: list of actions
  - `id`, `name`
  - `settings`: list of `{ key, label, type }` (type: `string|boolean|number`)
- executable path:
  - either `executable` (all platforms), or
  - `executable_linux` / `executable_windows`

### Invocation contract (current MVP)

When a key is pressed (key-down), RiverDeck-Redux spawns the plugin executable and writes a single JSON line to stdin:

- `plugin_id`
- `action_id`
- `event` (currently `key_down`)
- `key` (key index)
- `settings` (JSON object)

If the process exits non-zero, the UI surfaces an error.

## Data directories

Data is stored using `directories::ProjectDirs` for the app ID `io/github/riverdeck-redux`.

- **Profiles**: `<data_dir>/profiles/*.json`
- **Plugins**: `<data_dir>/plugins/<plugin_id>/...`

The exact `<data_dir>` depends on platform (e.g. Linux XDG data dir; Windows AppData).

## License

GPL-3.0-or-later. See `LICENSE.md`.

