# Copilot Instructions for rust-dashboard (termbookman)

## Project Overview

**rust-dashboard** (binary: `termbookman`) is a terminal UI application for managing commands, shell sessions, and GitHub Gist integration. It provides an interactive dashboard with a command sidebar, live terminal output with VT100 parsing, and Gist synchronization.

### Stack
- **TUI**: ratatui + crossterm (cross-platform terminal events)
- **Shell**: portable-pty (PTY abstraction) + vt100 parser (ANSI escape sequences)
- **Async**: tokio, futures, mpsc channels
- **APIs**: reqwest (GitHub REST API), serde/toml (config)

---

## Build & Runtime

### Build Commands
```bash
# Debug build
cargo build

# Release build (optimized, used for deployment)
cargo build --release

# Build & run
./build.sh && ./termbookman

# Build for ARM
./build.arm.sh
```

### Configuration
- **Config file**: `~/.config/termbookman/config.toml` (TOML format, loaded in [config.rs](src/config.rs))
- **Local commands**: `commands.txt` (in same directory as executable)
- **Cached gists**: `~/.config/termbookman/gists/` (with mtime metadata)

### Environment
- Set `TERM=xterm-256color` for proper color rendering
- GitHub API requires `GITHUB_TOKEN` or GitHub app device flow authentication

---

## Architecture

### Module Responsibilities

| Module | File | Responsibility |
|--------|------|---|
| **App State** | [app.rs](src/app.rs) | Central state struct, command loading, sidebar mode management, message dispatch |
| **UI Rendering** | [ui/mod.rs](src/ui/mod.rs) | Layout calculation, frame rendering |
| | [ui/terminal.rs](src/ui/terminal.rs) | Terminal output rendering using vt100 cells |
| | [ui/sidebar.rs](src/ui/sidebar.rs) | Sidebar list rendering (Commands/History/Gists modes) |
| | [ui/statusbar.rs](src/ui/statusbar.rs) | Status bar with configurable items |
| | [ui/modals.rs](src/ui/modals.rs) | Modal dialogs (confirmation, auth, upload progress) |
| **Event Handling** | [handlers.rs](src/handlers.rs) | Mouse/keyboard routing, modal interactions |
| **Config** | [config.rs](src/config.rs) | TOML parsing, color/statusbar/action definitions |
| **GitHub** | [github.rs](src/github.rs) | Non-blocking gist fetch/create/update via reqwest |
| **Utilities** | [utils.rs](src/utils.rs) | Parsing (commands, scripts), color conversion, formatting |
| **Main** | [main.rs](src/main.rs) | Event loop, PTY initialization, thread management |

### Data Flow

```
PTY Shell Output
    ↓
vt100 Parser (thread-safe, Arc<Mutex>)
    ↓
Terminal Cells (cached)
    ↓
Terminal Renderer (ui/terminal.rs)
    ↓
Ratatui Frame Draw

User Input (Mouse/Keyboard)
    ↓
Crossterm Event → Message enum
    ↓
Handlers (handlers.rs) → Mutate App State
    ↓
Next Render Cycle
```

### Message-Driven Architecture

The central `Message` enum (app.rs) coordinates all async operations and user input:

```rust
pub enum Message {
    Event(Event),           // Keyboard/mouse from crossterm
    PtyData,                // PTY output ready
    Tick,                   // 1s timer tick
    FetchGists,             // Gist sync initiated
    GistsFetched(...),      // Gists loaded (async callback)
    GistUploadStatus(...)   // Gist upload result
    // ... device auth flow, errors
}
```

**Threading Model**:
- Main thread: Event loop (`rx.recv()` blocks waiting for messages)
- PTY reader thread: Sends `PtyData` message when shell output available
- Event thread: Sends `Event(KeyEvent/MouseEvent)` messages
- Ticker thread: Sends `Tick` messages every 1 second
- GitHub thread: Spawned via tokio for non-blocking gist operations

---

## Key Design Patterns & Conventions

### 1. **Patched Dependency**
The vt100 crate is patched locally:
```toml
[patch.crates-io]
vt100 = { path = "patch/vt100" }
```
Modifications are in `patch/vt100/src/`. Always build against the patched version.

### 2. **Thread-Safe Parser State**
PTY parser and write access wrapped in `Arc<Mutex<>>`:
```rust
parser: Arc<Mutex<vt100::Parser>>
```
**Gotcha**: Never hold mutex locks across I/O operations (PTY writes, file reads). Lock → mutate → unlock → I/O.

### 3. **Sidebar Modes**
Three dynamic list modes (Commands, History, Gists) switched via `SidebarMode` enum:
- Commands: From `commands.txt`
- History: Previously executed commands
- Gists: Cached GitHub gists (with sync/upload)

Only one mode active; list state managed in App struct.

### 4. **PTY Resizing**
When terminal is resized, call:
```rust
master.resize(PtySize { rows, cols, ... })?;
```
**Failure to resize = garbled/wrapped output in child shell.**

### 5. **Configuration-Driven Actions**
Actions (mouse clicks, key bindings) defined in config.toml; handlers.rs queries config to dispatch behavior. Allows remappable controls.

### 6. **Non-Blocking GitHub Operations**
Gist fetch/upload spawned in tokio thread, result sent back via Message channel. Main thread never blocks on network I/O.

---

## Common Development Tasks

### Adding a New Sidebar Mode
1. Add variant to `SidebarMode` enum (app.rs)
2. Create new list state in App struct
3. Add rendering function in `ui/sidebar.rs`
4. Wire up mode switching in `handlers.rs`
5. Add toggle keybinding in config or handlers

### Adding a Modal Dialog
1. Add bool flag to App struct (e.g., `show_new_dialog: bool`)
2. Create render function in `ui/modals.rs`
3. Add modal rect calculation and rendering call in `ui/mod.rs`
4. Handle mouse/keyboard in `handlers.rs` to mutate dialog state

### Extending GitHub Integration
1. Add new Message variant in app.rs
2. Implement API call in github.rs (use reqwest client)
3. Spawn tokio thread in main.rs or github.rs
4. Send result back via `tx.send(Message::...)`
5. Handle result in app state update

### Modifying VT100 Rendering
Check [patch/vt100/src/](patch/vt100/src/) for:
- `cell.rs`: Cell attributes (color, bold, etc.)
- `parser.rs`: ANSI escape sequence parsing
- `screen.rs`: Screen state and cell grid
- `perform.rs`: Terminal control sequences (CSI, OSC, etc.)

### Testing
```bash
cargo test                  # Run all unit tests
cargo clippy                # Lint (style/correctness)
cargo fmt -- --check         # Format check
cargo build --target aarch64-unknown-linux-gnu  # Cross-compile test
```

---

## Common Issues & Solutions

| Issue | Cause | Solution |
|-------|-------|----------|
| Terminal output garbled/wrapped incorrectly | PTY not resized after terminal size change | Ensure `master.resize()` called in size change handler |
| Colors not rendering | TERM env var not set or unsupported | Set `export TERM=xterm-256color` before running |
| "Text file busy" error on build | Binary running while build.sh tries to replace it | Kill termbookman process before rebuilding |
| Gists not syncing | GitHub token invalid or network timeout | Check auth token in config, verify network connectivity |
| Parser crashes on escape sequences | Unsupported ANSI sequence in child shell | Report to vt100 crate or add handler in `patch/vt100/src/perform.rs` |
| Mutex deadlock on PTY write | Lock held across blocking I/O | Refactor to unlock before I/O operations |

---

## Code Style & Conventions

- **Naming**: `snake_case` for functions/variables, `PascalCase` for types/enums
- **Error handling**: Use `?` operator; main returns `Result<(), Box<dyn Error>>`
- **Comments**: Document why (intent), not what (code is clear). Link related issues/PRs.
- **Async**: Prefer tokio for I/O (`tokio::spawn`), avoid blocking waits
- **Testing**: Unit tests inline (`#[cfg(test)]`), integration tests in `tests/` folder

---

## Related Documentation

- [VT100 Parser README](patch/vt100/README.md) — ANSI sequence support and cell state
- [ratatui Book](https://ratatui.rs) — TUI widget/layout patterns
- [Crossterm Docs](https://docs.rs/crossterm) — Terminal event handling
- [Portable PTY](https://docs.rs/portable-pty) — Cross-platform shell spawning
