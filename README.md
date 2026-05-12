# tuxedo

A fast, keyboard-driven terminal UI for [todo.txt](http://todotxt.org/).
Vim-style bindings, atomic writes, instant external-edit detection, and four
hand-tuned themes — all in a single static binary.

```sh
brew install webstonehq/tap/tuxedo
```

[![CI](https://github.com/webstonehq/tuxedo/actions/workflows/ci.yml/badge.svg)](https://github.com/webstonehq/tuxedo/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/webstonehq/tuxedo?logo=github)](https://github.com/webstonehq/tuxedo/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](#license)
[![Rust](https://img.shields.io/badge/rust-2024-orange.svg?logo=rust)](https://www.rust-lang.org)

![tuxedo demo](docs/demo.gif)

## Highlights

- **Pure todo.txt.** Reads and writes the [standard format](https://github.com/todotxt/todo.txt) — every line is plain text you can edit with anything else.
- **Natural-language add.** Type prose into the add prompt — `Pay rent monthly on the first, show 3 days before due, project home` — and tuxedo rewrites it to canonical todo.txt for you to review and save. Local, offline, no AI service.
- **Vim keys, no surprises.** `j` / `k` to move, `dd` to delete, `gg` / `G` to jump, `u` to undo (50 levels), chord prompts (`gg`, `dd`, `fp`, `fc`) with a 600 ms window.
- **Command palette.** `:` or `Ctrl-P` opens a fuzzy palette over every action — type a few letters, hit Enter. Same matcher as `/` search, ranked so start-of-label hits beat word-boundary hits beat mid-word hits.
- **Atomic, sync-friendly writes.** Every change goes through write-temp-then-rename. If another process — Dropbox, an editor, a script — modifies the file, tuxedo reloads on the next keypress (or within ~250 ms while idle) and flashes a notice.
- **Sibling-file archive.** `A` moves completed tasks to `done.txt` next to your file, atomically.
- **Filter, sort, multi-select.** Cycle by `+project` or `@context`, sort by priority / due / file order, and bulk-complete or bulk-delete in visual mode.
- **Four themes, three densities.** Cycle with `T` and `D`. Choices persist across runs.
- **No daemon, no database, no cloud.** One file in, one file out.

## Screens

| | |
| --- | --- |
| **Empty state** • cell-bowtie mark and quick-start when the file has no tasks | ![empty](docs/screenshots/empty.svg) |
| **List** • list of todos, optionally grouped | ![empty](docs/screenshots/list.svg) |
| **Archive** • completed tasks grouped by completion date | ![archive](docs/screenshots/archive.svg) |
| **Filter sidebar active** • `fp` cycles projects with j/k, `fc` cycles contexts | ![filter](docs/screenshots/filter.svg) |
| **Command palette** • `:` or `Ctrl-P` opens a fuzzy palette over every action | ![command palette](docs/screenshots/command-palette.svg) |
| **Help** • `?` opens the full keybindings overlay | ![help](docs/screenshots/help.svg) |

<details>
    <summary>How to generate the screenshots and demo</summary>
    <p>The screenshots in the table above are checked-in SVGs. Regenerate them with:</p>
    <pre>mise run screenshots</pre>
    <p>The hero GIF at the top is recorded with <a href="https://github.com/charmbracelet/vhs">vhs</a> from <code>docs/demo.tape</code>. Regenerate it with:</p>
    <pre>mise run demo</pre>
</details>

## Themes

`T` cycles through four built-in themes.

| Muted Slate (default) | Dawn |
| --- | --- |
| ![muted slate](docs/screenshots/theme-muted-slate.svg) | ![dawn](docs/screenshots/theme-dawn.svg) |
| **Nord** | **Matrix** |
| ![nord](docs/screenshots/theme-nord.svg) | ![matrix](docs/screenshots/theme-matrix.svg) |

## Install

### Homebrew (macOS, Linux)

```sh
brew install webstonehq/tap/tuxedo
```

### Prebuilt binaries

Download the archive for your platform from the [latest release](https://github.com/webstonehq/tuxedo/releases/latest) and put `tuxedo` on your `PATH`.

Targets: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`. Each archive ships with a `.sha256` checksum.

### From source

```sh
cargo install --git https://github.com/webstonehq/tuxedo
```

Or clone and build:

```sh
git clone https://github.com/webstonehq/tuxedo
cd tuxedo
cargo build --release
./target/release/tuxedo [FILE]
```

Requires the Rust 2024 edition (recent stable toolchain).

## Usage

```sh
tuxedo [FILE]      # open FILE (created if missing)
tuxedo             # open ./todo.txt, or a sample file if none
tuxedo --sample    # open the bundled sample file in the temp dir
tuxedo update      # print upgrade instructions for your install
tuxedo --help
tuxedo --version
```

When a newer release is available, the status bar shows `↑ <version> (tuxedo
update)` next to the version. The check runs in the background, is cached at
`$XDG_CACHE_HOME/tuxedo/latest_version.json` for 24 h, and fails silently
when offline. Set `TUXEDO_NO_UPDATE_CHECK=1` to disable.

If `FILE` is omitted, tuxedo opens `./todo.txt` from the current working
directory if it exists. Otherwise it falls back to a sample todo.txt in the
system temp directory so you can poke around without committing to a path.

Edits are persisted on every change via atomic write (write `.tmp`, rename).

If the file changes on disk (another editor, a sync client, a script),
tuxedo notices on the next keypress, or within ~250 ms while idle, and
reloads. The keystroke that triggered the reload is consumed — press it
again to act on the fresh state — and the status bar flashes a notice.

Pressing `A` appends every completed task to a sibling `done.txt` and
removes them from the working file (atomically: `done.txt` is written
before the originals are dropped). `a` toggles the archive view so you
can browse, un-archive, or permanently delete past tasks.

## Keybindings

### Navigation

| Key | Action |
| --- | --- |
| `j` / `↓` | next task |
| `k` / `↑` | previous task |
| `gg` | first task |
| `G` | last task |
| `Ctrl-d` / `Ctrl-u` | half-page down / up |

### Editing

| Key | Action |
| --- | --- |
| `n` | add task |
| `e` / `i` | edit current task |
| `x` | toggle complete |
| `dd` | delete task |
| `p` | cycle priority A → B → C → · |
| `c` | add or remove a context |
| `+` | add a project |
| `yy` | copy current line to clipboard |
| `yb` | copy current body only (no priority, dates, projects, contexts, `key:value`) |
| `u` | undo (50 levels) |

### Filtering, sort, view

| Key | Action |
| --- | --- |
| `/` | search |
| `fp` | filter by project (`j` / `k` cycles, `Esc` clears) |
| `fc` | filter by context (`j` / `k` cycles, `Esc` clears) |
| `S` | cycle sort: priority → due → file order |
| `v` | enter visual / multi-select; `space` toggles a row |
| `x` / `dd` (in visual) | bulk-complete / bulk-delete the selection |
| `l` | list (default) view |
| `a` | toggle archive view |
| `A` | archive completed tasks → `done.txt` |
| `H` | toggle showing done tasks in the main list |

### Layout & theme

| Key | Action |
| --- | --- |
| `[` | toggle filter sidebar |
| `]` | toggle detail sidebar |
| `T` | cycle theme |
| `D` | cycle density: compact → comfortable → cozy |
| `L` | toggle line numbers |

### System

| Key | Action |
| --- | --- |
| `:` / `Ctrl-P` | command palette |
| `?` | help overlay |
| `,` | settings overlay |
| `q` | quit |

Two-key chord prompts (`gg`, `dd`, `yy`, `yb`, `fp`, `fc`) show a `g…` /
`d…` / `y…` / `f…` indicator in the status-bar mode chip while the
leader is armed; the window is 600 ms.

Copy uses the OSC 52 terminal escape, so it works locally and over SSH on
any terminal that supports it (kitty, alacritty, wezterm, iTerm2, foot,
modern xterm; tmux when `set -g set-clipboard on`). Older terminals will
silently ignore the keystroke.

## todo.txt format

Standard [todo.txt](https://github.com/todotxt/todo.txt) lines:

```
(A) 2026-04-28 Call dentist @phone +health due:2026-05-08
```

- `(A)` — priority, A through Z (omit for none)
- `2026-04-28` — creation date in ISO 8601
- `+project` — project tag
- `@context` — context tag
- `key:value` — extension; `due:YYYY-MM-DD` is recognized for sort and
  due-bucket grouping in the list view
- `rec:[+]N{d,b,w,m,y}` — recurrence; on completion (`x`), tuxedo inserts
  a fresh copy of the task with `due:` advanced by `N` days, business
  days (Mon–Fri), weeks, months, or years. The `+` prefix means
  *strict* recurrence anchored to the previous due date (e.g.
  `rec:+1m` for monthly rent on the 15th); without it, the new due is
  computed from the completion date (e.g. `rec:1w` for "water plants
  one week after I last did").

Completed tasks are prefixed with `x ` and a completion date:

```
x 2026-05-05 2026-05-01 Submit expense report +work
```

Recurring example:

```
2026-05-09 Pay rent due:2026-05-15 rec:+1m
```

Pressing `x` on the line above marks the original complete *and* inserts
`2026-05-09 Pay rent due:2026-06-15 rec:+1m`. `u` undoes both at once.

## Natural-language add

Press `n` to open the add prompt. Type the task in plain English. When the
buffer contains recognized phrases (dates, weekdays, recurrence, project /
context names, priority), pressing Enter rewrites the draft into canonical
todo.txt — review or tweak it, then Enter again to save.

| What you type | What lands in the draft |
| --- | --- |
| `Pay rent monthly on the first of the month, show the todo 3 days before the due date. It's part of project home and context bank` | `Pay rent +home @bank due:2026-06-01 rec:+1m t:-3d` |
| `Buy milk tomorrow` | `Buy milk due:2026-05-12` |
| `Call mom every week starting Friday for project family` | `Call mom +family due:2026-05-15 rec:+1w` |
| `Submit timesheet every other friday show 1 day before` | `Submit timesheet due:2026-05-15 rec:+2w t:-1d` |
| `Daily standup high priority` | `(A) standup rec:+1d` |
| `Annual review April 15 +work @office` | `Annual review +work @office due:2027-04-15` |

Recognized vocabulary:

- **Dates** — `today`, `tonight`, `tomorrow`, `yesterday`, weekdays (`monday` / `mon` …), months (`april 15`, `15th of april`), `in 3 days`, `the first of the month`, ISO `2026-05-15`.
- **Recurrence** — `daily`, `weekly`, `biweekly`, `monthly`, `yearly`, `annually`, `every monday`, `every 2 weeks`, `every other friday`, `every business day`.
- **Threshold** — `show 3 days before due`, `2 weeks before due`.
- **Projects / contexts** — prose form `project home` and `context bank`, or the standard `+home` / `@bank` sigils.
- **Priority** — `high priority` → A, `medium priority` → B, `low priority` → C, or `priority A`.

Parsing is rule-based and runs locally — no network calls, no API key. If
the buffer already contains a `due:`, `rec:`, or `t:` token, tuxedo assumes
you've typed canonical form and saves it directly on the first Enter.

## Configuration

Persisted to `${XDG_CONFIG_HOME:-$HOME/.config}/tuxedo/config.toml`. Cycling
theme, density, or sort, and toggling sidebars / line-numbers / done-visibility
all update the file. Unknown keys are ignored, so older binaries don't break
on newer files.

## Development

```sh
mise run fmt      # cargo fmt --all
mise run clippy   # cargo clippy --all-targets --locked -- -D warnings
mise run test     # cargo test --locked
```

CI runs all three on every push and pull request. Tasks are also runnable as
plain `cargo` commands if you don't use [mise](https://mise.jdx.dev/).

## Acknowledgments

- [todo.txt](http://todotxt.org/) by Gina Trapani — the format that makes a tool like this possible.
- [ratatui](https://ratatui.rs/) and [crossterm](https://github.com/crossterm-rs/crossterm) — the rendering and terminal-input crates tuxedo is built on.

## Roadmap

Planned and in-flight work lives in [`todo.txt`](./todo.txt) — eat your own dog food.

## Contributing

Issues and pull requests are welcome. For larger changes, please open an
issue first to discuss the approach. Run `mise run fmt clippy test` (or the
plain cargo equivalents) before submitting.

## License

Released under the [MIT License](https://opensource.org/licenses/MIT).
