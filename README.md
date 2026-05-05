# yt-music-tui

Download YouTube playlists into Apple Music from a terminal TUI.
Parallel downloads, live progress, resume, dedupe, library cleanup.

## Install

```sh
brew install yt-dlp ffmpeg deno
git clone git@github.com:NeoPrimate/music.git
cd music
cargo build --release
```

Binary: `./target/release/yt-music-tui`.

## Before first run

1. **Music.app → Settings → Files**: enable
   *"Copy files to Music Media folder when adding to library."*
   The app refuses to start without it.
2. **YouTube cookies** are required (anti-bot). Either:
   - log in to YouTube in **Firefox** and run with `--browser firefox`, or
   - log in in **Safari** and grant your terminal *Full Disk Access*
     (System Settings → Privacy & Security → Full Disk Access).

## Run

```sh
./target/release/yt-music-tui                    # defaults
./target/release/yt-music-tui --browser firefox  # recommended
./target/release/yt-music-tui -c <channel-url> -j 5 --browser firefox
```

| flag                  | default                                                |
|-----------------------|--------------------------------------------------------|
| `-c, --channel <URL>` | `https://www.youtube.com/@NeotenicApe/playlists`       |
| `-j, --parallelism N` | `3`                                                    |
| `--browser <NAME>`    | `safari` · `firefox` · `chrome` · `edge` · `none`      |
| `--ytdlp <PATH>`      | `/opt/homebrew/bin/yt-dlp`                             |
| `--ffmpeg <PATH>`     | `/opt/homebrew/bin/ffmpeg`                             |

## Keys

**Global:** `q` quit · `Tab` / `1` / `2` switch tabs · `r` refresh

**Download tab:**
`↑↓` / `jk` move · `Space` select · `a` select all · `Enter` / `d` download

**Library tab:**
`↑↓` / `jk` move · `Space` select · `a` select all
`x` delete playlist (tracks stay) · `X` delete playlist + its tracks

In a confirm modal: `y` yes · `n` / `Esc` no.

## Status badges

| badge                            | meaning                                     |
|----------------------------------|---------------------------------------------|
| (none)                           | not in Music yet                            |
| `📚 in library (N)`              | Music already has this playlist (N tracks) |
| `⟳ N/M`                          | downloading / importing                     |
| `✓ N/M imported`                 | complete                                    |
| `◐ N/M imported (partial)`       | some videos failed (private / unavailable) |
| `✓ already up to date`           | nothing new since last run                  |
| `✗ <message>`                    | failed (yt-dlp error shown)                 |

## How it works

1. Each download writes to its own `TempDir`. As yt-dlp finishes a track,
   it streams `IMPORT\t<file>\t<video_id>` on stdout.
2. The TUI immediately runs an AppleScript `add` to put that file in the
   matching Music.app user playlist (creating it if missing), then deletes
   the staging file. Music.app keeps its own copy in the Media folder.
3. Each track's YouTube video ID is embedded in the m4a `comment` tag, so
   re-running a playlist seeds yt-dlp's `--download-archive` from the
   existing Music tracks — already-imported videos are skipped.
4. Interrupt at any time. What was already imported is safe in Music; only
   the in-flight track is lost. Re-run picks up where it stopped.

## Troubleshooting

**`✗ no tracks downloaded — ERROR: ... Sign in to confirm you're not a bot`**
You need cookies. Use `--browser firefox` (after logging in via Firefox)
or grant the terminal Full Disk Access for Safari cookies.

**`Music.app's 'Copy files to Media folder ...' is OFF`**
Music → Settings → Files → check that box.

**Tracks downloaded but resume keeps re-downloading**
The YouTube-ID-in-comment tag isn't being applied. Verify in Music.app:
right-click a track → Get Info → Comments. Should show an 11-char video ID.
