use crate::events::DownloadEvent;
use color_eyre::eyre::{eyre, Result, WrapErr};
use serde::Deserialize;
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

    let v: serde_json::Value = serde_json::from_slice(&output.stdout)
        .wrap_err("failed to parse yt-dlp JSON")?;

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
    status: Option<String>,
    downloaded: Option<f64>,
    total: Option<f64>,
    speed: Option<f64>,
    eta: Option<u64>,
    index: Option<u32>,
    total_tracks: Option<u32>,
    title: Option<String>,
}

const PROGRESS_TEMPLATE: &str = r#"download:PROGRESS:{"status":%(progress.status)j,"downloaded":%(progress.downloaded_bytes)j,"total":%(progress.total_bytes)j,"speed":%(progress.speed)j,"eta":%(progress.eta)j,"index":%(info.playlist_index)j,"total_tracks":%(info.playlist_count)j,"title":%(info.title)j}"#;

pub async fn download_playlist(
    url: String,
    title: String,
    config: DownloadConfig,
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
        .arg("--newline")
        .arg("--no-colors")
        .arg("--progress-template")
        .arg(PROGRESS_TEMPLATE)
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
    let mut last_track_done: u32 = 0;

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

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                let _ = tx.send(DownloadEvent::Failed {
                    playlist_title: title.clone(),
                    message: "cancelled".into(),
                });
                return;
            }
            line = reader.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        if let Some(payload) = line.strip_prefix("PROGRESS:") {
                            if let Ok(p) = serde_json::from_str::<ProgressLine>(payload) {
                                let percent = match (p.downloaded, p.total) {
                                    (Some(d), Some(t)) if t > 0.0 => ((d / t) * 100.0) as f32,
                                    _ => 0.0,
                                };
                                let track_index = p.index.unwrap_or(0);
                                let track_total = p.total_tracks.unwrap_or(0);
                                let track_title = p.title.unwrap_or_default();
                                let status_s = p.status.as_deref().unwrap_or("");

                                if status_s == "finished" && track_index > last_track_done {
                                    last_track_done = track_index;
                                    let _ = tx.send(DownloadEvent::TrackDone {
                                        playlist_title: title.clone(),
                                        track_index,
                                        track_total,
                                    });
                                }

                                let _ = tx.send(DownloadEvent::Progress {
                                    playlist_title: title.clone(),
                                    track_index,
                                    track_total,
                                    track_title,
                                    percent,
                                    speed: p.speed,
                                    eta: p.eta,
                                });
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        }
    }

    let _ = child.wait().await;
    let _ = stderr_task.await;

    // Collect .m4a files even if yt-dlp partially failed.
    let files = match std::fs::read_dir(staging.path()) {
        Ok(dir) => {
            let mut v: Vec<PathBuf> = dir
                .filter_map(|e| {
                    let e = e.ok()?;
                    let p = e.path();
                    (p.extension().and_then(|s| s.to_str()) == Some("m4a")).then_some(p)
                })
                .collect();
            v.sort();
            v
        }
        Err(_) => Vec::new(),
    };

    if files.is_empty() {
        let log = stderr_log.lock().expect("stderr log poisoned");
        let last_error = log
            .iter()
            .rev()
            .find(|l| l.starts_with("ERROR:"))
            .cloned()
            .or_else(|| log.last().cloned())
            .unwrap_or_else(|| "(no stderr output)".to_string());
        let _ = tx.send(DownloadEvent::Failed {
            playlist_title: title,
            message: format!("no tracks downloaded — {last_error}"),
        });
    } else {
        let _ = tx.send(DownloadEvent::Finished {
            playlist_title: title,
            files,
            staging: Arc::new(staging),
        });
    }
}
