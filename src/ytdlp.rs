use crate::events::DownloadEvent;
use crate::music;
use color_eyre::eyre::{eyre, Result, WrapErr};
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use serde::Deserialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct PlaylistEntry {
    pub title: String,
    #[allow(dead_code)]
    pub id: String,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct DownloadConfig {
    pub ytdlp: PathBuf,
    pub ffmpeg: PathBuf,
    pub browser: String,
}

pub async fn list_playlists(channel_url: &str, ytdlp: &Path) -> Result<Vec<PlaylistEntry>> {
    let output = Command::new(ytdlp)
        .args([
            "--flat-playlist",
            "-J",
            "--no-warnings",
            "--ignore-errors",
            channel_url,
        ])
        .output()
        .await
        .wrap_err_with(|| format!("failed to spawn yt-dlp at {}", ytdlp.display()))?;

    if output.stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(eyre!("yt-dlp produced no output: {}", stderr));
    }

    let v: serde_json::Value =
        serde_json::from_slice(&output.stdout).wrap_err("failed to parse yt-dlp JSON")?;

    let entries = v
        .get("entries")
        .and_then(|e| e.as_array())
        .ok_or_else(|| eyre!("unexpected yt-dlp shape: missing `entries`"))?;

    let mut playlists = Vec::new();
    for e in entries {
        let id = e.get("id").and_then(|s| s.as_str()).unwrap_or("").to_string();
        if id.is_empty() {
            continue;
        }
        let title = e
            .get("title")
            .and_then(|s| s.as_str())
            .unwrap_or("(untitled)")
            .to_string();
        let url = e
            .get("url")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("https://www.youtube.com/playlist?list={}", id));
        playlists.push(PlaylistEntry { title, id, url });
    }

    Ok(playlists)
}

#[derive(Deserialize)]
struct ProgressLine {
    downloaded: Option<f64>,
    total: Option<f64>,
    speed: Option<f64>,
    eta: Option<u64>,
    index: Option<u32>,
    total_tracks: Option<u32>,
    title: Option<String>,
}

const PROGRESS_TEMPLATE: &str = r#"download:PROGRESS:{"status":%(progress.status)j,"downloaded":%(progress.downloaded_bytes)j,"total":%(progress.total_bytes)j,"speed":%(progress.speed)j,"eta":%(progress.eta)j,"index":%(info.playlist_index)j,"total_tracks":%(info.playlist_count)j,"title":%(info.title)j}"#;

/// Print line emitted after each track's m4a is finalized at its output path.
const PRINT_TEMPLATE: &str = "after_move:IMPORT\t%(filepath)s\t%(id)s";

pub async fn download_playlist(
    url: String,
    title: String,
    config: DownloadConfig,
    archive_seed: Vec<String>,
    tx: mpsc::UnboundedSender<DownloadEvent>,
    cancel: CancellationToken,
) {
    let staging = match TempDir::new() {
        Ok(d) => d,
        Err(e) => {
            let _ = tx.send(DownloadEvent::Failed {
                playlist_title: title,
                message: format!("staging dir: {e}"),
            });
            return;
        }
    };

    // Write the download archive (resume support).
    let archive_path = staging.path().join("download-archive.txt");
    if !archive_seed.is_empty() {
        match std::fs::File::create(&archive_path) {
            Ok(mut f) => {
                for id in &archive_seed {
                    let _ = writeln!(f, "youtube {id}");
                }
            }
            Err(e) => {
                let _ = tx.send(DownloadEvent::Failed {
                    playlist_title: title,
                    message: format!("archive: {e}"),
                });
                return;
            }
        }
    }

    let _ = tx.send(DownloadEvent::Started {
        playlist_title: title.clone(),
    });

    let output_template = format!(
        "{}/%(playlist_index)03d - %(title)s.%(ext)s",
        staging.path().display()
    );

    let mut cmd = Command::new(&config.ytdlp);
    cmd.arg("-x")
        .args(["--audio-format", "m4a"])
        .arg("--ffmpeg-location")
        .arg(&config.ffmpeg)
        .arg("--ignore-errors");
    if !config.browser.is_empty() && config.browser != "none" {
        cmd.arg("--cookies-from-browser").arg(&config.browser);
    }
    cmd.arg("--embed-metadata")
        .arg("--embed-thumbnail")
        .args(["--parse-metadata", "uploader:%(artist)s"])
        .args(["--parse-metadata", "playlist_title:%(album)s"])
        .arg("--no-quiet")
        .arg("--newline")
        .arg("--no-colors")
        .arg("--progress-template")
        .arg(PROGRESS_TEMPLATE)
        .arg("--print")
        .arg(PRINT_TEMPLATE)
        .arg("--download-archive")
        .arg(&archive_path)
        .arg("-o")
        .arg(&output_template)
        .arg(&url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(DownloadEvent::Failed {
                playlist_title: title,
                message: format!("spawn yt-dlp: {e}"),
            });
            return;
        }
    };

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    let mut reader = BufReader::new(stdout).lines();

    let stderr_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let stderr_log_writer = stderr_log.clone();
    let stderr_task = tokio::spawn(async move {
        let mut err_reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = err_reader.next_line().await {
            let mut log = stderr_log_writer.lock().expect("stderr log poisoned");
            log.push(line);
            if log.len() > 200 {
                let drop_n = log.len() - 200;
                log.drain(0..drop_n);
            }
        }
    });

    let mut imports: FuturesUnordered<
        tokio::task::JoinHandle<std::result::Result<String, (String, String)>>,
    > = FuturesUnordered::new();
    let mut imported_count: u32 = 0;
    let mut failed_count: u32 = 0;
    let mut tracks_expected: Option<u32> = None;
    let mut yt_done = false;

    loop {
        if yt_done && imports.is_empty() {
            break;
        }
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                let _ = tx.send(DownloadEvent::Failed {
                    playlist_title: title.clone(),
                    message: "cancelled".into(),
                });
                return;
            }
            line = reader.next_line(), if !yt_done => {
                match line {
                    Ok(Some(line)) => {
                        if let Some(payload) = line.strip_prefix("PROGRESS:") {
                            if let Ok(p) = serde_json::from_str::<ProgressLine>(payload) {
                                if let Some(t) = p.total_tracks { if t > 0 { tracks_expected = Some(t); } }
                                let percent = match (p.downloaded, p.total) {
                                    (Some(d), Some(t)) if t > 0.0 => ((d / t) * 100.0) as f32,
                                    _ => 0.0,
                                };
                                let _ = tx.send(DownloadEvent::Progress {
                                    playlist_title: title.clone(),
                                    track_index: p.index.unwrap_or(0),
                                    track_total: p.total_tracks.unwrap_or(0),
                                    track_title: p.title.unwrap_or_default(),
                                    percent,
                                    speed: p.speed,
                                    eta: p.eta,
                                });
                            }
                        } else if let Some(payload) = line.strip_prefix("IMPORT\t") {
                            // payload is "filepath\tvideo_id"
                            if let Some((filepath, vid)) = payload.split_once('\t') {
                                let file = PathBuf::from(filepath);
                                let vid = vid.to_string();
                                let title2 = title.clone();
                                let vid_for_task = vid.clone();
                                let task = tokio::spawn(async move {
                                    match music::add_file_to_playlist(&title2, &file).await {
                                        Ok(()) => {
                                            let _ = std::fs::remove_file(&file);
                                            Ok(vid_for_task)
                                        }
                                        Err(e) => Err((vid_for_task, e.to_string())),
                                    }
                                });
                                imports.push(task);
                            }
                        }
                    }
                    Ok(None) => { yt_done = true; }
                    Err(_) => { yt_done = true; }
                }
            }
            Some(joined) = imports.next() => {
                match joined {
                    Ok(Ok(vid)) => {
                        imported_count += 1;
                        let _ = tx.send(DownloadEvent::TrackImported {
                            playlist_title: title.clone(),
                            video_id: vid,
                        });
                    }
                    Ok(Err((vid, msg))) => {
                        failed_count += 1;
                        let _ = tx.send(DownloadEvent::TrackFailed {
                            playlist_title: title.clone(),
                            video_id: vid,
                            message: msg,
                        });
                    }
                    Err(_) => {
                        failed_count += 1;
                    }
                }
            }
        }
    }

    let _ = child.wait().await;
    let _ = stderr_task.await;

    if imported_count == 0 && failed_count == 0 {
        // yt-dlp ran but produced no IMPORT lines.
        let log = stderr_log.lock().expect("stderr log poisoned");
        let last_error = log
            .iter()
            .rev()
            .find(|l| l.starts_with("ERROR:"))
            .cloned()
            .or_else(|| log.last().cloned())
            .unwrap_or_else(|| "(no stderr output)".to_string());
        // archive_seed.is_empty() means there was nothing already imported,
        // so 0/0 really means failure. Otherwise it's "everything was already imported".
        if archive_seed.is_empty() {
            let _ = tx.send(DownloadEvent::Failed {
                playlist_title: title,
                message: format!("no tracks downloaded — {last_error}"),
            });
            return;
        }
    }

    let _ = tx.send(DownloadEvent::ImportDone {
        playlist_title: title,
        tracks_imported: imported_count,
        tracks_expected,
    });

    drop(staging);
}
