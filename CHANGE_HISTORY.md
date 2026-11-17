# Change History

Newest first.

## [2026-11-17] — feat(docker): container shell, actions and compose/run command view (Portainer-like)
- Issue #26 created on GitHub
- Scope: telepad-core (remote commands), telepad-app, telepad-gui (Docker section)
- Labels: enhancement — needs the owner's decision on relaxing the read-only rule

## [2026-11-17] — feat(collector): overview flags for reboot needed, pending updates and end-of-life OS
- Issue #25 created on GitHub
- Scope: telepad-core (script.sh, parse, model, EOL table), telepad-app, telepad-gui
- Labels: enhancement

## [2026-11-17] — fix(ui): choosing an IdentityFile on macOS; key permission hint (#23)
- Host editor: **Browse…** opens in the current key's folder or `~/.ssh` (hidden on
  macOS/Linux, so it was unreachable from the home folder), with a Cmd+Shift+. hint; an arrow
  next to the field lists the private keys in `~/.ssh` (`find_ssh_keys` reads only each
  file's first line; `.pub`, `known_hosts`, `config`, PuTTY `.ppk` are skipped).
- ssh's "UNPROTECTED PRIVATE KEY FILE … bad permissions" is now reported as
  "private key … is readable by other users … run: chmod 600 <key>" instead of
  "authentication failed" (reported on macOS with a 0644 key).
- Verified on Windows: the editor's key list offered the four OpenSSH keys in `~/.ssh` and
  skipped `.ppk`/`.pub`/text files; tests for key detection, the editor flow and the error
  text. Not verified on a Mac: the macOS dialog starting in `~/.ssh`.

## [2026-11-17] — feat(ui): Telepad app icon (#21)
- Original icon (rounded blue gradient square, white "T", green online dot) drawn by
  `scripts/make-icon.py`; `assets/icon/` holds PNG (256, 1024), `.ico` (16–256) and `.icns`.
- Windows: `build.rs` embeds the icon and version info in `telepad.exe`; installer uses it
  (`SetupIconFile`). All platforms: window icon via `ViewportBuilder::with_icon`.
  macOS: `Telepad.app/Contents/Resources/telepad.icns` + `CFBundleIconFile`. README header.
- Verified on Windows: icon extracted from the built exe, title-bar icon in the running
  app, file properties show "Telepad" 0.5.0.

## [2026-11-17] — Telepad 0.5.0
- Windows installer (`…-windows-x86_64-setup.exe`, per-user) next to the portable zip (#10).
- macOS `Telepad.app` in a `.dmg` per architecture, ad-hoc signed, not notarized (#9).
- Installed copies keep data in the per-user folder; portable zips keep it next to the
  program (`portable` marker).
- Overview: sortable, resizable table (#4) or cards (#16); bigger Overview button; Add host
  in the Hosts menu (#16); compact host list with grouping by status (#8).
- Fixes: detail header and top bar wrap (#5), missing-glyph boxes (#6), Windows `.sha256`
  line ending (#7); import of any host-list JSON and one-click `~/.ssh/config` import.

## [2026-11-17] — feat(release): macOS Telepad.app in a .dmg (#9)
- `package-unix.sh` on macOS also builds `Telepad.app` (Info.plist, both programs, licences),
  ad-hoc signs it (`codesign -s -`; no Apple Developer account, so not notarized) and packs
  it with an Applications link into `telepad-<ver>-macos-<arch>.dmg` (+ LF `.sha256`).
- No portable marker in the app: data in `~/Library/Application Support/Telepad`.
- Self-update inside a `.app` re-applies the ad-hoc signature after swapping the executable
  (`platform::after_self_update`), so the bundle stays valid.
- Release workflow mounts each `.dmg` read-only and checks the Applications link, both
  executables, no marker, `plutil -lint`, `codesign --verify`, `telepad-check --help`.
- Not verified on a real Mac: install by drag, first start through Gatekeeper, and an
  in-app update of the bundle (CI only).

## [2026-11-17] — feat(release): Windows installer alongside the portable zip (#10)
- `installer/telepad.iss` (Inno Setup 6): per-user install to `%LOCALAPPDATA%\Programs\Telepad`
  (no admin), Start-menu entry, optional desktop icon, uninstaller; data is kept.
- Data folder: `--db` / `TELEPAD_DB` > next to `config` > next to the exe when portable
  (`portable` marker, or an existing `telepad.db`) > per-user folder (`%APPDATA%\Telepad`,
  `~/Library/Application Support/Telepad`, `~/.local/share/telepad`). Zips ship the marker,
  so portable copies behave as before; `config` is also looked for in the data folder.
- `package-windows.ps1 -Installer` builds `telepad-<ver>-windows-x86_64-setup.exe` (+ LF
  `.sha256`); the release workflow installs Inno Setup, builds it, verifies every `.sha256`,
  and smoke-tests a silent install → `telepad-check --help` → silent uninstall.
- Verified locally: a copy with the marker created its database next to itself, a copy
  without it in `%APPDATA%\Telepad` (test folder removed afterwards); zip contains `portable`.

## [2026-11-17] — feat(ui): overview cards, bigger Overview button, Add host in Hosts menu (#16)
- Overview **Table | Cards** switch (saved as `ui_overview_cards`); cards wrap to the width,
  follow search and a **Sort by** picker (`Msg::SetSort`, `SortKey::ALL`).
- Large full-width **Overview — all hosts** button; Add host removed from the host list;
  Hosts menu grouped Add / Import / Export.
- Verified on Windows with 65 real hosts: button 38 units tall, cards in 11 rows, setting
  saved, menu items present. Sort-by picker driven with real mouse clicks on test hosts:
  inventory zeta,alpha,mike → Host alpha,mike,zeta → reverse zeta,mike,alpha.
- Manual (Overview: Table / Cards) and the packaged README.txt use the new menu labels.

## [2026-11-17] — tested: #8 passes
- Windows 11, release build, 65 real hosts (read-only): compact view by default; Group by
  status → Down (16), Up (49); Detailed + grouped stored as `ui_compact_host_list=0`,
  `ui_group_by_status=1` and restored after a restart. fmt, clippy, 62 tests, release
  build; diff scanned.

## [2026-11-17] — feat(ui): compact host list, group by status (#8)
- Host list: **Compact** view (default) — one line per host with the worst of CPU / RAM /
  disk on the right, details on hover — or the previous **Detailed** rows; optional
  **Group by status** (Down / Up / Not checked yet, collapsible, down first).
- Both choices are saved in the `settings` table (`ui_compact_host_list`,
  `ui_group_by_status`) via `Effect::SaveUiPrefs`, loaded with `Msg::Started`.
- Logic in `telepad-app` (`host_list_groups`, `worst_usage`, `Msg::SetCompactHostList`,
  `Msg::SetGroupByStatus`) with 3 new tests; store round-trip test in core.
- Verified on Windows against 65 real hosts (read-only): default compact; Group by status
  showed Down (16) and Up (49); Detailed + grouped saved as 0/1 in the database and restored
  after a restart.

## [2026-11-17] — tested: #7 passes
- Release dry run on PR #14: all five platform packages verified with `sha256sum -c` /
  `shasum -c` (no CR) — windows, linux x86_64/aarch64, macos x86_64/aarch64 "zip: OK".

## [2026-11-17] — fix(release): Windows .sha256 written with LF (#7)
- `package-windows.ps1` wrote the hash file with `Set-Content`, which ends the line with
  CRLF, so `sha256sum -c` looked for a file name ending in a carriage return and failed. It
  now writes `<hash>  <file>` with an LF ending and no BOM, like `sha256sum` does.
- Release workflow: every platform's package check fails on a CR in the `.sha256` and runs
  `sha256sum -c` (`shasum -a 256 -c` on macOS) against the zip.
- Verified on Windows: the new file ends with LF, has no CR or BOM, and `sha256sum -c` → OK.

## [2026-11-17] — tested: #6 passes
- Windows 11, release build: Docker stack headers on a real Docker host show
  "Host port → container" with the arrow (read-only collection); the glyph test covers every
  GUI string (incl. "→ Container" in Ports in use) and all icon constants, and fails without
  the fallback. fmt, clippy, 58 tests, release build; diff scanned.

## [2026-11-17] — fix(ui): "→" no longer renders as a box (#6)
- Cause: egui's proportional font (Ubuntu-Light + egui-icons) has no U+2192, so the
  `Host port → container` header in Docker stacks and the `→ Container` header in Ports in
  use drew an empty box (monospace port values were fine: Hack has it).
- Fix: `icons::definitions()` adds Hack as the last fallback of the proportional family, so
  any symbol the UI font lacks is drawn by the monospace font instead of a box.
- Guard: GUI test `every_character_in_the_ui_has_a_glyph` scans every string literal in
  `crates/telepad-gui/src` and every icon constant and checks both font families with the
  real font setup; it fails (naming file:line) without the fallback.
- Verified on Windows against a real Docker host (read-only): the headers show the arrow.

## [2026-11-17] — tested: #4 and #5 pass
- Windows 11, 200 % scaling, release build; documentation-IP test hosts (unreachable, so
  long errors fill the OS column). Automated with UI Automation and real mouse input.
- #4: Host sort asc / desc / third click restores inventory order; Address sorts
  numerically (192.0.2.9 < 192.0.2.10 < 192.0.2.30 < 198.51.100.2); dragging the
  Host|Description border 200 px moved the Description column 204 px; long description and
  error text cut with "…" inside their cells; CPU header stays in the window.
- #5: at the 1800 px (900-unit) minimum width every detail and top-bar button is inside the
  window (Delete and the loading count wrap to a second line).
- fmt, clippy, 57 tests and the release build pass; diff scanned for secrets.

## [2026-11-17] — fix(ui): detail header and top bar wrap instead of clipping (#5)
- Host detail header split into a wrapping identity row and a wrapping button row; order
  Open SSH/RDP, Copy ssh command, Refresh | Edit, Clone, Delete (destructive last).
- Top bar wraps too: at the 900-unit minimum width the loading count was cut off.
- Verified on Windows (200 % scaling) at 1800 px and 2560 px wide via UI Automation: every
  header and top-bar button lies inside the window; at minimum width Delete and the
  loading count move to a second line. Test used documentation-IP hosts only.

## [2026-11-17] — feat(ui): sortable, resizable overview columns (#4)
- Overview rebuilt with `egui_extras::TableBuilder`: click a header to sort (natural
  direction, reversed, then inventory order), caret icon shows the direction; drag header
  borders to resize; cells clip with "…" and show the full text on hover, so errors and
  long OS names no longer widen columns; sticky header, horizontal scroll when needed.
- Sorting lives in `telepad-app` (`table.rs`: `Sort`, `SortKey`, `overview_rows`, shared
  `fullest_disk`/`hottest`), changed only via `Msg::SortBy`; 4 new tests (click cycle,
  numeric columns, missing values last, numeric IP order + filter).
- Verified on Windows against 65 real hosts (read-only): CPU sort desc/asc/off and
  Address sort through UI Automation; screenshots checked for clipping and the arrow.
- Not verified by automation: dragging a column border (built into TableBuilder).

## [2026-11-17] — Telepad 0.4.1: macOS and Linux releases
- Release workflow builds and publishes windows-x86_64, linux-x86_64, linux-aarch64,
  macos-aarch64 and macos-x86_64 zips (+ `.sha256`) in one release; each zip is unpacked and
  checked (execute bits, `telepad-check --help`) before publishing. PRs touching release files
  run it as a dry run.
- `scripts/package-unix.sh` (macOS/Linux, optional cross target); `package-windows.ps1 -Publish`
  removed so a release always contains every platform.
- Fix: the self-updater now sets the execute bit on the new `telepad-check` on macOS/Linux
  (`platform::make_executable`); before, it would have become non-executable after an update.
- README: install steps for macOS (quarantine) and Linux (portal, terminals, glibc).
- Import is lenient: any JSON with a list of hosts (Telepad export of any version or app
  name, `{"hosts": [...]}`, a bare `[...]`, common field names); everything else is read as
  an OpenSSH config. New **Hosts → Import ~/.ssh/config** (no file dialog). Exports made
  before the rename import again.

## [2026-11-17] — refactor: Cargo workspace + Model–View–Update (#1)
- Workspace: `telepad-core` (inventory, ssh collection, parsers, platform, self-update; no GUI),
  `telepad-app` (State/Msg/Effect/`update()`; no GUI, no I/O), `telepad-gui` (egui views +
  runtime, binary `telepad`), `telepad-check` (CLI, no longer compiles eframe).
- `ui.rs` (2,400 lines) split into `views/` per screen, `widgets.rs`, `runtime.rs`, `app.rs`.
  Views read the model, edit drafts and return messages; `update()` is the only place state
  changes and is covered by 17 behaviour tests; the runtime is the only GUI code doing I/O.
- No behaviour change; same binaries, database and files. Docs (AGENTS, CONTRIBUTING, PR
  template) describe the rules; packaging script and CI updated for the workspace.

## [2026-11-17] — Telepad 0.4.0: first public release
- Published as Telepad (MIT OR Apache-2.0) at https://github.com/yohannaftali/telepad.
- Self-update from GitHub Releases: checks `releases/latest` at start and every 6 h,
  **Update now** downloads `telepad-<ver>-<os>-<arch>.zip`, verifies the `.sha256`,
  replaces `telepad` and `telepad-check`, offers Restart. `telepad-check --check-update`.
- Features carried over: SQLite host database with OpenSSH import/export, Docker stacks
  and container IPs/ports, ports in use, sensors incl. BMC via ipmitool, RDP-only hosts
  with ping status, OS logos, portable Windows zip.
- CI on Windows, Linux and macOS; tag-triggered release workflow.
