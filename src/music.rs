use color_eyre::eyre::{eyre, Result, WrapErr};
use std::path::Path;
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct LibraryPlaylist {
    pub name: String,
    pub track_count: u32,
}

fn applescript_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub async fn run_osascript(script: &str) -> Result<String> {
    let out = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .await
        .wrap_err("running osascript failed")?;
    if !out.status.success() {
        return Err(eyre!(
            "osascript failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
}

pub async fn check_copy_on_import() -> Result<()> {
    let out = Command::new("defaults")
        .args(["read", "com.apple.Music", "copy-files-to-library-on-import"])
        .output()
        .await
        .wrap_err("running `defaults read` failed")?;
    if out.status.success() {
        let val = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if val == "0" {
            return Err(eyre!(
                "Music.app's 'Copy files to Media folder when adding to library' is OFF.\n\
                 Enable it: Music → Settings → Files → check \"Copy files to Music Media folder \
                 when adding to library\". Then re-run."
            ));
        }
    }
    Ok(())
}

pub async fn list_user_playlists() -> Result<Vec<LibraryPlaylist>> {
    let script = r#"
tell application "Music"
    set output to ""
    repeat with p in user playlists
        try
            if not (smart of p) then
                set output to output & (name of p) & tab & (count of tracks of p) & linefeed
            end if
        end try
    end repeat
    return output
end tell
"#;
    let s = run_osascript(script).await?;
    let mut out = Vec::new();
    for line in s.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some((name, count_str)) = line.split_once('\t') {
            let count = count_str.trim().parse::<u32>().unwrap_or(0);
            out.push(LibraryPlaylist {
                name: name.to_string(),
                track_count: count,
            });
        }
    }
    Ok(out)
}

/// Returns the `comment` ID3 tag of every track in the named user playlist.
/// Used to seed yt-dlp's `--download-archive` so already-imported videos are skipped.
pub async fn playlist_track_comments(name: &str) -> Result<Vec<String>> {
    let escaped = applescript_escape(name);
    let script = format!(
        r#"
tell application "Music"
    set output to ""
    try
        set p to user playlist "{}"
        repeat with t in (every track of p)
            try
                set c to (comment of t)
                if c is not "" then
                    set output to output & c & linefeed
                end if
            end try
        end repeat
    end try
    return output
end tell
"#,
        escaped
    );
    let s = run_osascript(&script).await?;
    Ok(s.lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

/// Add a single file to a user playlist (creating the playlist if missing).
pub async fn add_file_to_playlist(name: &str, file: &Path) -> Result<()> {
    let escaped_name = applescript_escape(name);
    let escaped_path = applescript_escape(&file.to_string_lossy());
    let script = format!(
        r#"
tell application "Music"
    set playlistName to "{name}"
    if not (exists user playlist playlistName) then
        make new user playlist with properties {{name:playlistName}}
    end if
    set targetPlaylist to user playlist playlistName
    add (POSIX file "{path}") to targetPlaylist
end tell
"#,
        name = escaped_name,
        path = escaped_path
    );
    run_osascript(&script).await?;
    Ok(())
}

/// Delete the named user playlist (does NOT delete the underlying tracks).
pub async fn delete_user_playlist(name: &str) -> Result<()> {
    let escaped = applescript_escape(name);
    let script = format!(
        r#"
tell application "Music"
    try
        delete user playlist "{}"
    end try
end tell
"#,
        escaped
    );
    run_osascript(&script).await?;
    Ok(())
}

/// Delete a user playlist AND every track that was in it from the library
/// (frees disk space; tracks go to Music.app's trash subject to its settings).
pub async fn delete_playlist_and_tracks(name: &str) -> Result<()> {
    let escaped = applescript_escape(name);
    let script = format!(
        r#"
tell application "Music"
    try
        set p to user playlist "{}"
        set theIds to {{}}
        repeat with t in (every track of p)
            try
                set end of theIds to persistent ID of t
            end try
        end repeat
        delete p
        repeat with anId in theIds
            try
                delete (every track of library playlist 1 whose persistent ID is anId)
            end try
        end repeat
    end try
end tell
"#,
        escaped
    );
    run_osascript(&script).await?;
    Ok(())
}
