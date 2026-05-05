use color_eyre::eyre::{eyre, Result, WrapErr};
use std::path::PathBuf;
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

pub async fn import_playlist(name: &str, files: &[PathBuf]) -> Result<u32> {
    if files.is_empty() {
        return Ok(0);
    }
    let escaped_name = applescript_escape(name);
    let mut adds = String::new();
    for f in files {
        let p = applescript_escape(&f.to_string_lossy());
        adds.push_str(&format!(
            "    try\n        add (POSIX file \"{}\") to targetPlaylist\n    end try\n",
            p
        ));
    }
    let script = format!(
        r#"
tell application "Music"
    set playlistName to "{name}"
    if not (exists user playlist playlistName) then
        make new user playlist with properties {{name:playlistName}}
    end if
    set targetPlaylist to user playlist playlistName
{adds}
    return (count of tracks of targetPlaylist) as text
end tell
"#,
        name = escaped_name,
        adds = adds
    );
    let out = run_osascript(&script).await?;
    Ok(out.parse::<u32>().unwrap_or(files.len() as u32))
}
