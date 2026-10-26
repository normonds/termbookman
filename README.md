# termbookman

`termbookman` is a high-performance terminal dashboard built with Rust and the Ratatui TUI framework. It provides a specialized environment for developers, combining a live PTY terminal with an integrated command sidebar and GitHub Gist management.

Coded mostly by Google's Gemini 3

![Screenshot](helpers/screenshot.gif)

![License](https://img.shields.io/badge/license-MIT-blue.svg)
![Rust](https://img.shields.io/badge/rust-stable-brightgreen.svg)

## 🚀 Features

-   **Split-Pane Environment**: A live, functional PTY shell side-by-side with a powerful command sidebar.
-   **Multi-Source Sidebar**:
    -   **COMMANDS**: A local registry of common tasks and scripts defined in `commands.txt`.
    -   **HISTORY**: Instant access to your deduped shell history (latest 500 items).
    -   **GISTS**: Browse, search, edit, and sync your GitHub Gists directly from the TUI.
-   **Advanced Gist Integration**:
    -   Identify and execute scripts with shebangs (`#!`).
    -   Detect local modifications and prompt for GitHub uploads.
    -   Built-in Gist creation and deletion tools.
-   **Terminal Excellence**:
    -   Full 256-color support.
    -   Custom-patched `vt100` parser for accurate Alternate Character Set (ACS) rendering (ideal for tools like `btop`, `htop`, and `nvtop`).
    -   Mouse support for scrolling, selecting, and clicking.
-   **Cross-Platform ARM Support**: Dedicated build scripts for ARM64 (Ubuntu ARM) deployments.

## 🛠 Technical Stack

-   **Framework**: [Ratatui](https://github.com/ratatui/ratatui) + `crossterm`.
-   **PTY Bridge**: `portable-pty`.
-   **Terminal Parser**: A locally maintained patch of `vt100` for box-drawing character integrity.
-   **Event Loop**: Plain threads (PTY reader, input, timer, network jobs) feeding one `std::sync::mpsc` channel.

## 📦 Installation

### Prerequisites
-   Rust (stable)
-   `aarch64-linux-gnu-gcc` (for ARM64 builds)

### Native Build
```bash
./build.sh
```

### ARM64 Build
```bash
./build.arm.sh
```

## ⚙️ Configuration

Settings are managed in `~/.config/termbookman/config.toml`. The application will automatically create a default configuration on its first run, including:
-   External editor preference (e.g., `nano`, `vim`, `code`).
-   UI color schemes for the terminal and status bars.
-   GitHub authentication (Device Flow or PAT).
-   Customizable status bar items and actions.
-   `sync_commands_to_gist` (off by default): upload `commands.txt` to a private Gist whenever it changes.

`config.toml` stores the GitHub token and is written with owner-only permissions. The same directory holds `commands.txt`, the gist cache (`gists/<gist id>/<file>`) and `debug.log`. Set `TERMBOOKMAN_CONFIG_DIR` to use a different directory.

Fetching gists never overwrites local edits that were not uploaded yet; such files are listed in the status bar. Deleting a script removes only that file from its Gist (the Gist itself is deleted only when the file was its last one).

The in-app updater only installs a release binary whose `.sha256` checksum (published by CI next to each binary) matches.

## 🖥 Command Line

```bash
termbookman <label>             # run a command or script by its sidebar label
termbookman script <label>      # run a script entry
termbookman print <label>       # print a command
termbookman print script <label># print a script's source
termbookman run <file> <key>    # run the entry <key> from <file> (used by the sidebar)
termbookman exec '<cmd>'        # run a command, asking for its <prompt:Label:Default> values
termbookman --version
```

## ⌨️ Controls

-   **Mouse**: Click tabs, sidebar items, and modal buttons. Scroll terminal or sidebar.
-   **Ctrl-C**: Send interrupt to the active PTY process.
-   **Shift-PageUp / Shift-PageDown**: Scroll terminal history.
-   **Shift + mouse**: Select text even when the running program captures the mouse; **Ctrl-Shift-C** copies it.
-   **Settings Button**: Open the configuration modal to manage GitHub tokens and editor settings.

## 📄 License
This project is licensed under the MIT License.
