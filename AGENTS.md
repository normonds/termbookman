# AGENTS.md

Guidance for AI coding agents working in this repository.

## Project

termbookman is a Rust TUI (ratatui + crossterm) that shows a live bash PTY next to a sidebar
with three tabs: saved commands (`commands.txt`), shell history, and GitHub Gists. The same
binary is also a CLI that runs sidebar entries. The crate is named `rust-dashboard`; the built
binary is copied to `./termbookman` (x86_64) or `./tbm.arm` (aarch64).

## Commands

```bash
cargo build                    # debug build
cargo test                     # unit tests (all in-module, no network)
cargo clippy --all-targets     # lint; keep it free of new warnings
./build.sh                     # release build -> ./termbookman
./build.arm.sh                 # aarch64 cross build -> ./tbm.arm (needs aarch64-linux-gnu-gcc)
```

Run the TUI against a throwaway config so you never touch the user's real one:

```bash
TERMBOOKMAN_CONFIG_DIR=/tmp/tbm-cfg ./target/debug/rust-dashboard
```

To drive it non-interactively, run it in `tmux` and use `send-keys`; mouse clicks can be sent
as raw SGR sequences, e.g. `tmux send-keys -t s -l $'\e[<0;83;4M'` (1-based column;row).

## Architecture

There is no async runtime. Threads (PTY reader, input events, 1 s tick, network jobs) send
`app::Message` values into one `std::sync::mpsc` channel; `main.rs` drains the queue, handles
each message in `handle_message`/`handle_event`, then redraws once.

| File | Responsibility |
|------|----------------|
| `main.rs` | Terminal setup/teardown, PTY spawn, event loop, message handling |
| `cli.rs` | CLI mode: `<label>`, `script`, `print`, `run <file> <key>`, `exec <cmd>` |
| `app.rs` | `App` state, stats/git refresh, edit detection, selection copy |
| `entry.rs` | `Entry` (one sidebar row) and loading/parsing of commands files and cached gists |
| `gists.rs` | Local gist cache, sync manifest, `fetch_all` |
| `github.rs` | GitHub REST calls and device-flow login (blocking `reqwest`) |
| `input.rs` | Key/mouse/paste -> PTY byte sequences |
| `handlers.rs` | Mouse handling for modals, status bars, sidebar, terminal pane |
| `updater.rs` | Self-update with SHA-256, ELF and test-run verification |
| `config.rs` | `config.toml` schema, defaults, config dir resolution |
| `utils.rs` | Logging, file writing helpers, quoting, truncation, parsers |
| `ui/` | Rendering only: `terminal`, `sidebar`, `statusbar`, `modals` |
| `patch/vt100/` | Vendored `vt100` 0.16.2 with line-drawing charset (ACS) support in `perform.rs` |

## Rules that are easy to break

- **Never write into the current working directory.** Logs go to `utils::log_path()` (config
  dir); config, `commands.txt` and the gist cache live under `config::get_config_dir()`.
  CLI invocations run inside the user's shell, wherever they happen to be.
- **Write files through `utils::write_file` / `write_private_file`.** They create parent dirs
  and fix ownership when running under sudo. `config.toml` holds the GitHub token and must
  stay `0600` (`write_private_file`).
- **Never log secrets or gist contents**, and don't log full HTTP response bodies.
- **Slice strings by characters, not bytes.** Use `utils::truncate_to_width` / `display_width`
  for anything rendered; byte slicing panics on non-ASCII text.
- **Quote anything typed into the PTY** with `utils::shell_quote` (paths, keys, exe path).
- **The sidebar runs entries by `termbookman run <file> <key>`** (`handlers::run_line`), not by
  label. Labels are display-only and deduplicated across sources; `key` is the label within
  its own file. Keep `entry::parse_file_entries` the single place that builds entries.
- **Sidebar selection indexes the filtered list.** Use `App::filtered_indices()` /
  `selected_entry()`; don't index `entries()` with the selection directly.
- **Status bar layout is shared.** Rendering and click handling both use
  `ui::statusbar::bar_layout`; change sizing there only.
- **Gist sync must not lose data.** Fetches go through `gists::store_remote_file`, which keeps
  unuploaded local edits (manifest hash differs). Record successful uploads with
  `Manifest::record_synced`. Only call GitHub delete when the gist id is known, and use
  `github::delete_gist_file`, which removes a single file unless it is the gist's last one.
- **Don't block the UI thread** on network or long work; spawn a thread and send a `Message`.
- **Restore the terminal on every exit path.** `main::restore_terminal` is also called from the
  panic hook.

## Testing

Tests live in `#[cfg(test)] mod tests` at the bottom of each module. Prefer extracting pure
functions (as in `input.rs`, `github.rs` body/plan helpers, `gists::decide`) and testing those.
Use `utils::test_dir(name)` for a fresh temp directory. Tests must not use the network, the
user's config dir, or process-global env vars (tests run in parallel).

## Releases

Every push to `master` makes CI build both targets, run the tests, and publish a GitHub release
with `tbm`, `tbm.arm` and their `.sha256` files. The in-app updater refuses binaries without a
matching checksum, so keep the checksum step. Do work on a branch and merge to `master`
only when a release is intended.

## Conventions

- Match the surrounding style; keep comments for the non-obvious "why".
- Commit messages: conventional prefixes (`feat:`, `fix:`, `refactor:`, `docs:`, `ci:`).
- Don't commit binaries, `debug.log`, `helpers/github.pat.txt` or anything under `target/`.
- Older notes in `helpers/` and `.github/copilot-instructions.md` may be out of date; trust the
  code.
