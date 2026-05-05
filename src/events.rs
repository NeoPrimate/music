use crate::music::LibraryPlaylist;
use crate::ytdlp::PlaylistEntry;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

#[derive(Debug)]
pub enum DownloadEvent {
    Started {
        playlist_title: String,
    },
    Progress {
        playlist_title: String,
        track_index: u32,
        track_total: u32,
        track_title: String,
        percent: f32,
        speed: Option<f64>,
        eta: Option<u64>,
    },
    TrackDone {
        #[allow(dead_code)]
        playlist_title: String,
        #[allow(dead_code)]
        track_index: u32,
        #[allow(dead_code)]
        track_total: u32,
    },
    Finished {
        playlist_title: String,
        files: Vec<PathBuf>,
        staging: Arc<TempDir>,
    },
    ImportDone {
        playlist_title: String,
        tracks_imported: u32,
    },
    Failed {
        playlist_title: String,
        message: String,
    },
}

#[derive(Debug)]
pub enum AppEvent {
    Download(DownloadEvent),
    LibraryRefreshed(Vec<LibraryPlaylist>),
    PlaylistsRefreshed(Vec<PlaylistEntry>),
    BackgroundError(String),
}
