# TODO

Suggestions from a full read of `src/` (all 67 tests pass; `cargo clippy --all-targets` shows 5
warnings; `cargo fmt --check` reports 54 diffs). Ordered by priority within each section.
Line numbers are as of commit `cf62770`.

## P0: data loss / correctness bugs

- [x] **A broken `config.toml` gets overwritten with defaults.** `config::load_config`
      (`src/config.rs:302`) logs the parse error and returns `DEFAULT_CONFIG`. The next
      `save_config` (entering a PAT, editor or update URL, or device login) writes the defaults
      back over the user's hand-edited file, which loses their status bar layout and maybe their
      token. Fix: remember that parsing failed, show it in the status bar, and refuse to save
      (or first back the file up to `config.toml.bak`).
- [x] **An upload can overwrite an unrelated gist.** `github::upload_gist` → `resolve_gist_id`
      (`src/github.rs:131`) picks *the first gist that has a file with that name* when no id is
      known. On the first `commands.txt` sync, any existing gist that contains a
      `commands.txt` (from another tool or project) gets overwritten. This is the same danger
      AGENTS.md already rules out for deletes. Fix: when the id is unknown, always create a new
      gist (or only match gists whose description is `Uploaded via termbookman`).
- [x] **No conflict check before uploading.** Auto-upload PATCHes over whatever is on GitHub,
      so edits made on another machine since the last sync are silently lost. Before PATCHing,
      compare the remote content's hash with the manifest's `hash`. If they differ, keep both
      versions (e.g. upload as `name.conflict-<host>`) or ask the user.
- [x] **Sidebar runs break after a self-update.** After `updater::install` renames the new
      binary over the running one, `std::env::current_exe()` returns `".../termbookman (deleted)"`
      on Linux. `handlers::exe_word` (`src/handlers.rs:20`) then types a path that doesn't exist,
      so every sidebar click fails until restart. Fix: resolve the exe path once at startup and
      store it in `App`, or strip the ` (deleted)` suffix.
- [x] **Auto-fetch at startup switches tabs.** `Message::GistsFetched` always sets
      `sidebar_mode = Gists` and resets the selection (`src/main.rs:540`). With a token
      configured, the startup fetch pulls the user off the Commands tab a second or two after
      launch. Only switch when the user started the fetch (e.g. the FETCH button).
- [x] **Full HTTP error bodies are logged.** `upload_gist` puts the whole response body into its
      error (`src/github.rs:150`), and `spawn_upload` then writes it to `debug.log` and the
      status bar. This breaks the AGENTS.md rule. Keep the status code and at most the
      `message` field from the JSON.
- [x] **Duplicate display labels.** `utils::dedup_label` (`src/utils.rs:326`) can produce a
      label that already exists. For example, entries labelled `ls`, `ls`, `ls2` come out as
      `ls`, `ls2`, `ls2`. Then `termbookman <label>` and `print` resolve to the wrong entry. Keep
      incrementing until the label is unused.
- [x] **A panic in the PTY reader thread kills the UI.** The parser `Mutex` is locked with
      `.unwrap()` everywhere (e.g. `src/main.rs:140`, `src/ui/terminal.rs:11`). If `vt100`
      panics on odd input, the mutex is poisoned and the next draw panics too. Use
      `lock().unwrap_or_else(|e| e.into_inner())`, or wrap `process` in `catch_unwind`.

## P1: blocking the UI thread / performance

- [x] **`git` runs synchronously on the UI thread every 4 s.** `App::update_stats` calls
      `git_info_for` (`src/app.rs:423-426`), which runs `git rev-parse` and `git status
      --porcelain`. In a large repo or on a slow disk, `git status` takes seconds and freezes
      input and rendering. Move it to a worker thread that sends a `Message::GitInfo`. Also
      consider `git status --porcelain -uno` or `--untracked-files=no` for speed.
- [x] **`sys.refresh_processes()` scans every process each second** (`src/app.rs:372`) only to
      check whether the shell has a child. `portable_pty::MasterPty::process_group_leader()`
      gives the PTY's foreground process group directly. Busy means
      `leader != shell_pid`. It's cheaper, and it's more accurate: background jobs
      (`sleep 100 &`) no longer lock the sidebar. It also gives the foreground program's name,
      which could go in the status bar. Refresh only CPU and memory for the stats item.
- [x] **Gist fetching is N+1 and sequential.** `gists::fetch_all` (`src/gists.rs:195-220`)
      downloads every file's `raw_url` one at a time. Store each gist's `updated_at` in the
      manifest and skip unchanged gists. Download the rest in parallel (a small thread pool),
      and reuse one `reqwest` client instead of building one per call (`github::download_text`).
- [x] **`filtered_indices()` re-filters and re-lowercases every entry several times per frame**
      (sidebar render, `selected_entry()` in the status bar, click handling). Cache the
      filtered list in `App` and invalidate it when the query, tab or entries change.

## P1: gist sync gaps

- [x] **Remotely deleted gists stay in the cache forever.** After a fetch, remove cached
      `<gist_id>/` folders that are no longer on GitHub, but only when their content matches
      the manifest hash, so unuploaded edits are kept.
- [x] **A failed download drops the file from the Gists tab** even though a cached copy
      exists (`src/gists.rs:213-216`). Fall back to the cached file and mark it as stale.
- [x] **The Gists tab is empty until a fetch finishes** (e.g. offline), while the Commands tab
      already shows the cached gists (`src/entry.rs:197`). Fill `gist_entries` from the cache at
      startup, and decide whether cached gists belong in the Commands tab at all, since they
      currently show up in both tabs.
- [x] **`commands.txt` sync only goes one way.** It uploads, but the remote copy is never
      merged back into the local file (`load_entries` skips the cached copy). Either document
      it as a backup, or pull remote changes using the same `decide` logic.
- [x] **Edits made outside the sidebar are never uploaded.** Only files opened by
      double-click or created with "+ NEW SCRIPT" are tracked (`editing_file` holds a single
      path). On each tick, or before a fetch, compare cached files with their manifest hash,
      show an "N unsynced" indicator, and offer an upload action.
- [x] **The local file is deleted before the remote delete succeeds** (`src/main.rs:436`). If
      the GitHub call fails, the next fetch brings the file back. Delete locally only after the
      remote delete succeeds, or say clearly that the remote delete failed and the file will
      reappear.
- [x] Use `Accept: application/vnd.github+json` plus `X-GitHub-Api-Version`, and turn
      403/429 rate-limit responses into a readable message.

## P2: UX

- [x] **Add keyboard navigation for the sidebar.** Today every key goes to the PTY unless the
      search box has focus, so the sidebar is mouse-only. Suggestion: a focus toggle key such
      as `F2` or `Ctrl+Space`, `↑/↓/PgUp/PgDn` to move, `Enter` to run, `e` to edit, `Del` to
      delete, `Tab` to switch tabs. In search, `Enter` should run the selected match rather
      than only closing search.
- [x] Make the run target bigger than the 2-column `▸` area (`x <= 1` in
      `handle_list_mouse`), or run on single click anywhere on the row and edit on a modifier
      or double-click. Also make sidebar resizing discoverable (it's Ctrl+click-drag on the
      terminal scrollbar).
- [x] **Mask the PAT in the settings modal** (`src/ui/modals.rs:128`): it's shown in plain
      text. Show `ghp_••••last4` unless the field has focus. Add a "Log out / clear token"
      action, because an empty PAT can't be saved right now.
- [x] Opening Settings without a token starts the device flow immediately (a network call
      plus a new code each time). Use an explicit "Log in with GitHub" button instead.
- [x] **History tab:**
      - It reads `$HOME/.bash_history` (`src/app.rs:322`), which is root's under sudo. Use the
        SUDO_USER home, as `config::get_config_dir` already does.
      - bash writes history only on exit, so commands typed in the embedded shell don't appear.
        Export `PROMPT_COMMAND='history -a'` in the spawned shell, e.g. via `--rcfile`
        wrapping the user's `.bashrc`.
      - Respect `$HISTFILE`.
      - Add an action to save a history line to `commands.txt`.
- [x] Ask for `<prompt:Label:Default>` values in a TUI modal instead of stdin inside the
      shell. It would show the defaults, allow editing, and skip the `termbookman exec`
      round-trip.
- [x] Copying needs a fallback: `arboard` fails over SSH and on headless servers
      (`src/app.rs:442`), which are likely targets given the ARM and sudo support. Fall back to
      OSC 52 (`\x1b]52;c;<base64>\x07`) written to the outer terminal. Show success or failure
      in the status bar instead of only in the log.
- [x] Mouse selection uses viewport coordinates, so it drifts when new output scrolls the
      screen or when the user scrolls back. Anchor it to absolute line numbers (scrollback
      offset plus row). Also add double-click to select a word and triple-click to select a
      line.
- [x] Text inputs (`main::edit_text`) only support append and backspace. Add cursor movement
      with ←/→, Home/End and Delete, plus Ctrl+U and Ctrl+W.
- [x] Use fuzzy matching (e.g. `nucleo` or `fuzzy-matcher`) and sort by frecency instead of
      substring filtering.
- [x] Make the hard-coded menu (`btop`, `df -h` in `handlers::handle_menu_key` and
      `ui/modals.rs`) configurable in `config.toml`, the same way the status bar buttons are.
- [x] Status bar buttons choose black or white text by matching the color *name*
      (`green`/`cyan`/`light_*`), so hex colors always get white text. Pick the text color
      from the color's luminance.
- [x] System stats round memory to whole GB, so `0/1GB` shows on small ARM boards. Show one
      decimal place below 10 GB.
- [x] Prevent concurrent self-updates (double-clicking UPDATE starts two threads). Compare the
      build id with the release before downloading, so the button can say "Already up to date".

## P2: shell / PTY environment

- [x] `LANG=C.UTF-8` is forced (`src/main.rs:119`), which overrides the user's locale. Set it
      only when the current locale isn't UTF-8.
- [x] The shell is hard-coded to `bash` (`src/main.rs:116`). Add a `shell` config key that
      defaults to `$SHELL`, falling back to bash. The CLI's `bash -c` could stay as it is.
- [x] Export `TERMBOOKMAN=1` (and the binary path) in the spawned shell, so scripts and
      prompts can detect it and the sidebar doesn't need the full exe path.
- [x] Make the 1000-line scrollback (`src/main.rs:127`) configurable.
- [x] Missing SGR attributes are worth checking in the vendored vt100: strikethrough, and
      blink rendered as bold. Also consider sending `Event::FocusGained/Lost` (DECSET 1004)
      to programs that request it.

## P2: robustness / security hygiene

- [x] Write files atomically (temp file + `rename`) in `utils::write_file` and
      `write_private_file` (`src/utils.rs:110`). A crash or a full disk mid-write currently
      corrupts `.manifest.json`, `config.toml` or a script.
- [x] Don't ignore errors: `let _ = save_config(...)` (5 places in `main.rs`),
      `let _ = write_file(...)` when creating the default `commands.txt`, and
      `clipboard.set_text`. Show them in the status bar.
- [x] `save_config` rewrites `config.toml` with `toml` and drops the user's comments and key
      order. Use `toml_edit` to change only the edited key.
- [ ] Optional: read the token from `GH_TOKEN`/`GITHUB_TOKEN`, from `gh auth token`, or from the
      OS keyring instead of keeping it in plain text in `config.toml`.
- [ ] The updater's checksum comes from the same origin as the binary, so it catches
      corruption but not a compromised release. Consider signing releases (minisign or
      cosign) and checking the signature, or at least pinning the update host.
- [x] Under sudo, `updater::install` (`src/updater.rs:107`) creates the replacement binary as
      root. Keep the original file's owner and mode.
- [x] `parse_lines` skips *any* line that contains `# termbookman`, including commands such as
      `echo "# termbookman"`. Match only a whole comment line.

## P3: code structure

- [x] **Share modal and sidebar layouts the way `statusbar::bar_layout` does.** The settings
      modal constraint list is duplicated in `handlers.rs:187-206` and `ui/modals.rs:49-68`. The
      delete-confirm layout and the sidebar's tabs/search/list/new-button rectangles are
      duplicated the same way, as is `is_sidebar_locked` (`ui/sidebar.rs:20` vs
      `handlers.rs:360`). One `fn settings_layout(area) -> SettingsRects` per screen removes the
      chance that clicks and rendering drift apart. Add a rule to AGENTS.md.
- [ ] Replace the boolean flags in `App` (`show_menu`, `show_settings_modal`,
      `show_new_gist_dialog`, `show_delete_confirm`, `is_pat_focused`,
      `is_update_url_focused`, `is_editor_focused`, `is_search_focused`) with
      `enum Modal { None, Menu, Settings(Field), NewGist, DeleteConfirm(PathBuf) }` and
      `enum Focus`. Impossible states go away and `focused_input` / `handle_event` get simpler.
- [ ] Split `handle_message` (≈250 lines in `main.rs`) into per-feature functions, such as
      `gists::on_message` and `auth::on_message`. Move the upload/delete thread bodies out of
      `main.rs`.
- [ ] Group `App`'s 60 public fields into sub-structs: `TerminalState`, `SidebarState`,
      `AuthState`, `SyncStatus`.
- [x] Find entries by path in one helper (the
      `gist_entries.iter().chain(sidebar_entries)…find(path)` lookup is repeated 3 times in
      `main.rs`).

## P3: tooling, CI, release

- [x] Fix the 5 `clippy::manual_is_multiple_of` warnings: use `(now / 500).is_multiple_of(2)`
      in `ui/modals.rs` and `ui/sidebar.rs`, or a shared `blink_on(app)` helper.
- [x] Run `cargo fmt` (54 diffs), then add `cargo fmt --check` and
      `cargo clippy --all-targets -- -D warnings` to CI.
- [x] Release tags are `v<YYYY-MM-DD>`, so two pushes on the same day reuse the tag and the
      release ends up pointing at stale assets or failing. Use `v<date>.<run_number>` or
      include the short SHA.
- [x] Put the git SHA in `--version` (from `build.rs`) and expose `CARGO_PKG_VERSION`. Then
      the updater can tell when it's already on the latest build, and bug reports can name the
      exact build. Honor `SOURCE_DATE_EPOCH` for reproducible builds.
- [x] Add `[profile.release] strip = true, lto = "thin", codegen-units = 1` for smaller
      binaries, which also means faster self-updates.
- [ ] Upgrade dependencies that are well behind: `ratatui 0.26`, `crossterm 0.27`,
      `sysinfo 0.30`, `reqwest 0.11`, `directories 5`. Ratatui's `f.size()` and
      `set_cursor` are deprecated in newer versions.
- [x] Offer the ACS/HVP/scroll-region patches in `patch/vt100` upstream, so the vendored fork
      can eventually be dropped. Delete `patch/vt100/Cargo.toml.orig`.
- [x] Run the aarch64 tests in CI (via `cross` or qemu-user); they're currently skipped.
- [x] Repository cleanup:
      - The tracked `Link to termbookman` entry looks like a stray desktop symlink.
      - Add the untracked `.antigravitycli/` to `.gitignore`.
      - `helpers/issues.txt` and `.github/copilot-instructions.md` are out of date (AGENTS.md
        says as much). Fold what's still true into AGENTS.md and delete the rest.
      - Consider renaming the crate from `rust-dashboard` to `termbookman`, so the build
        scripts don't need to copy and rename the binary.

## Tests to add

- [x] Use ratatui's `TestBackend` to render a frame and assert on sidebar rows and
      status bar text, and to check that clicks hit the rectangles that were drawn.
- [x] Config parse failure: a test that a bad `config.toml` is not overwritten (after the P0
      fix).
- [x] `dedup_label` collision case (`ls`, `ls`, `ls2`).
- [x] Gist id resolution: an upload with an unknown id never PATCHes an existing gist.
- [x] `run_line` keeps working when `current_exe` ends in ` (deleted)`.
