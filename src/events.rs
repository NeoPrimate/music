use crate::music::LibraryPlaylist;
use crate::ytdlp::PlaylistEntry;

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
    /// One file finished download+postprocess and was successfully imported into Music.app.
    TrackImported {
        playlist_title: String,
        #[allow(dead_code)]
        video_id: String,
    },
    /// One file finished download+postprocess but failed to import.
    TrackFailed {
        #[allow(dead_code)]
        playlist_title: String,
        #[allow(dead_code)]
        video_id: String,
        #[allow(dead_code)]
        message: String,
    },
    /// Final terminal state for a playlist run.
    ImportDone {
        playlist_title: String,
        tracks_imported: u32,
        tracks_expected: Option<u32>,
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
