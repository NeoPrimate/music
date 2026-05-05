# yt-music-tui

A Rust TUI that downloads YouTube playlists into Apple Music with parallel
downloads, live progress bars, and library management.

## What it does

- Lists all playlists from a YouTube channel's `/playlists` page.
- Multi-select playlists, download in parallel (configurable concurrency).
- Each YouTube playlist becomes a same-named user playlist in Music.app
  (cover art + metadata embedded).
- A second tab lists existing Music.app user playlists; multi-select delete
  with a confirmation modal.

## Requirements

macOS (Apple Silicon tested) and the following Homebrew packages:

```sh
brew install yt-dlp ffmpeg deno
```

`deno` is required by yt-dlp's current YouTube extractor.

Music.app must have **"Copy files to Media folder when adding to library"**
enabled (Music → Settings → Files). The app refuses to start if it's off.

### YouTube cookies

YouTube's anti-bot check requires authenticated cookies for most downloads.
The default is `--browser safari`, which means yt-dlp reads cookies from
Safari. On macOS this requires your terminal (Alacritty / Terminal /
iTerm / etc.) to have **Full Disk Access**:

System Settings → Privacy & Security → Full Disk Access → add your terminal
app.

Alternatives:

- `--browser firefox` — Firefox stores cookies in a user-readable location,
  no Full Disk Access needed. Just be logged in to YouTube in Firefox.
- `--browser chrome` — same idea.
- `--browser none` — skip cookies entirely. Only works for playlists
  YouTube doesn't bot-check.

## Build

```sh
cargo build --release
```

The binary lands at `./target/release/yt-music-tui`.

## Usage

```sh
yt-music-tui [OPTIONS]
```

Options:

| flag                  | default                                                  |
|-----------------------|----------------------------------------------------------|
| `-c, --channel <URL>` | `https://www.youtube.com/@NeotenicApe/playlists`         |
| `-j, --parallelism N` | `3`                                                      |
| `--ytdlp <PATH>`      | `/opt/homebrew/bin/yt-dlp`                               |
| `--ffmpeg <PATH>`     | `/opt/homebrew/bin/ffmpeg`                               |
| `--browser <NAME>`    | `safari` (also: `firefox`, `chrome`, `edge`, `none`)     |

Examples:

```sh
# default channel, 3-way parallel, Safari cookies
yt-music-tui

# custom channel, 5 concurrent downloads, Firefox cookies
yt-music-tui -c 'https://www.youtube.com/@SomeChannel/playlists' -j 5 --browser firefox
```

## Keys

Global:

| key             | action       |
|-----------------|--------------|
| `q` / `Ctrl-C`  | quit         |
| `Tab` / `1`/`2` | switch tab   |
| `r`             | refresh      |

Download tab:

| key            | action                               |
|----------------|--------------------------------------|
| `↑↓` / `j` `k` | move cursor                          |
| `Space`        | toggle selection                     |
| `a`            | toggle select all                    |
| `Enter` / `d`  | download selected (skips in-flight)  |

Library tab:

| key            | action                          |
|----------------|---------------------------------|
| `↑↓` / `j` `k` | move cursor                     |
| `Space`        | toggle selection                |
| `a`            | toggle select all               |
| `x` / `Del`    | delete selected (with confirm)  |

In the delete confirm modal: `y` confirm, `n` / `Esc` cancel.

## How it works

1. On startup, runs `yt-dlp --flat-playlist -J <channel>` to enumerate
   playlists.
2. Selecting playlists and pressing Enter spawns one tokio task per
   playlist, gated by an `Arc<Semaphore>` so at most `--parallelism`
   downloads run concurrently.
3. Each download task spawns yt-dlp with a JSON progress template
   (`--progress-template`) and parses progress events line-by-line into
   percent / speed / ETA / current track.
4. The playlist downloads to a per-task `TempDir` (`$TMPDIR/.tmpXXXX/`).
   yt-dlp's `--ignore-errors` keeps it going past unavailable / private
   videos; partial successes are still imported.
5. When download finishes, status flips to `Importing` and an AppleScript
   (`osascript`) creates (or finds) the user playlist and adds each
   `.m4a` file to it. Music.app's "copy to Media folder" setting takes
   over, so the staging dir is dropped immediately after import.
6. Library tab is refreshed automatically after each successful import.

## Status badges

| badge          | meaning                                |
|----------------|----------------------------------------|
| `⏳ queued`    | waiting on a parallelism slot          |
| `⟳`            | downloading                            |
| `⇒♫`           | downloading done, importing to Music   |
| `✓ N imported` | done, N tracks in the Music playlist   |
| `✗ <message>`  | failed (yt-dlp's error is shown)       |

## Troubleshooting

**`✗ no tracks downloaded — ERROR: ... Sign in to confirm you're not a bot`**
→ See the cookies section above. Either grant terminal Full Disk Access
for Safari, or use `--browser firefox`.

**Pre-flight fails with "Copy files to Media folder ... is OFF"**
→ Music → Settings → Files → check the "Copy files to Music Media folder
when adding to library" box.

**Downloaded but not in Music yet**
→ Tracks only appear in Music.app after the *entire* playlist finishes
downloading and the status badge flips to `✓ N imported`. Three
parallel playlists means three full downloads complete first.
