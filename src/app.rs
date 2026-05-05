use crate::events::{AppEvent, DownloadEvent};
use crate::music::{self, LibraryPlaylist};
use crate::ytdlp::{self, DownloadConfig, PlaylistEntry};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::{mpsc, Semaphore};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Download,
    Library,
}

#[derive(Debug, Clone)]
pub enum DownloadStatus {
    /// We saw a Music.app playlist with the same name on startup; not yet (re-)run.
    InLibrary {
        tracks_in_music: u32,
    },
    Queued,
    Running {
        track_index: u32,
        track_total: u32,
        track_title: String,
        percent: f32,
        speed: Option<f64>,
        eta: Option<u64>,
        imported_so_far: u32,
    },
    Done {
        tracks_imported: u32,
        tracks_expected: Option<u32>,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone)]
pub enum ConfirmKind {
    DeletePlaylists(Vec<String>),
    DeletePlaylistsAndTracks(Vec<String>),
}

pub struct App {
    pub tab: Tab,
    pub playlists: Vec<PlaylistEntry>,
    pub library: Vec<LibraryPlaylist>,
    pub download_cursor: usize,
    pub download_selected: HashSet<usize>,
    pub library_cursor: usize,
    pub library_selected: HashSet<usize>,
    pub statuses: HashMap<String, DownloadStatus>,
    pub confirm: Option<ConfirmKind>,
    pub status_msg: Option<String>,
    pub should_quit: bool,

    pub channel_url: String,
    pub config: DownloadConfig,
    pub sem: Arc<Semaphore>,
    pub app_tx: mpsc::UnboundedSender<AppEvent>,
    pub cancel: CancellationToken,
}

impl App {
    pub fn new(
        playlists: Vec<PlaylistEntry>,
        library: Vec<LibraryPlaylist>,
        channel_url: String,
        config: DownloadConfig,
        parallelism: usize,
        app_tx: mpsc::UnboundedSender<AppEvent>,
        cancel: CancellationToken,
    ) -> Self {
        let mut s = Self {
            tab: Tab::Download,
            playlists,
            library,
            download_cursor: 0,
            download_selected: HashSet::new(),
            library_cursor: 0,
            library_selected: HashSet::new(),
            statuses: HashMap::new(),
            confirm: None,
            status_msg: None,
            should_quit: false,
            channel_url,
            config,
            sem: Arc::new(Semaphore::new(parallelism.max(1))),
            app_tx,
            cancel,
        };
        s.sync_startup_statuses();
        s
    }

    fn sync_startup_statuses(&mut self) {
        let by_name: HashMap<&str, &LibraryPlaylist> = self
            .library
            .iter()
            .map(|lp| (lp.name.as_str(), lp))
            .collect();
        for p in &self.playlists {
            if let Some(lp) = by_name.get(p.title.as_str()) {
                self.statuses.insert(
                    p.title.clone(),
                    DownloadStatus::InLibrary {
                        tracks_in_music: lp.track_count,
                    },
                );
            }
        }
    }

    pub fn handle_terminal_event(&mut self, event: Event) {
        let key = match event {
            Event::Key(k) if k.kind == KeyEventKind::Press => k,
            _ => return,
        };

        if self.confirm.is_some() {
            self.handle_modal_key(key);
            return;
        }

        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), _) => {
                self.should_quit = true;
                return;
            }
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.should_quit = true;
                return;
            }
            (KeyCode::Tab, _) => {
                self.tab = match self.tab {
                    Tab::Download => Tab::Library,
                    Tab::Library => Tab::Download,
                };
                return;
            }
            (KeyCode::Char('1'), _) => {
                self.tab = Tab::Download;
                return;
            }
            (KeyCode::Char('2'), _) => {
                self.tab = Tab::Library;
                return;
            }
            (KeyCode::Char('r'), _) => {
                self.refresh();
                return;
            }
            _ => {}
        }

        match self.tab {
            Tab::Download => self.handle_download_key(key),
            Tab::Library => self.handle_library_key(key),
        }
    }

    fn handle_modal_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => match self.confirm.take() {
                Some(ConfirmKind::DeletePlaylists(names)) => {
                    self.execute_delete(names, false);
                }
                Some(ConfirmKind::DeletePlaylistsAndTracks(names)) => {
                    self.execute_delete(names, true);
                }
                None => {}
            },
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.confirm = None;
            }
            _ => {}
        }
    }

    fn handle_download_key(&mut self, key: KeyEvent) {
        let n = self.playlists.len();
        if n == 0 {
            return;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.download_cursor > 0 {
                    self.download_cursor -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.download_cursor + 1 < n {
                    self.download_cursor += 1;
                }
            }
            KeyCode::Char(' ') => {
                if !self.download_selected.insert(self.download_cursor) {
                    self.download_selected.remove(&self.download_cursor);
                }
            }
            KeyCode::Char('a') => {
                if self.download_selected.len() == n {
                    self.download_selected.clear();
                } else {
                    self.download_selected = (0..n).collect();
                }
            }
            KeyCode::Enter | KeyCode::Char('d') => {
                self.start_downloads();
            }
            _ => {}
        }
    }

    fn handle_library_key(&mut self, key: KeyEvent) {
        let n = self.library.len();
        if n == 0 {
            return;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.library_cursor > 0 {
                    self.library_cursor -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.library_cursor + 1 < n {
                    self.library_cursor += 1;
                }
            }
            KeyCode::Char(' ') => {
                if !self.library_selected.insert(self.library_cursor) {
                    self.library_selected.remove(&self.library_cursor);
                }
            }
            KeyCode::Char('a') => {
                if self.library_selected.len() == n {
                    self.library_selected.clear();
                } else {
                    self.library_selected = (0..n).collect();
                }
            }
            KeyCode::Char('x') | KeyCode::Delete => {
                let names = self.selected_library_names();
                if !names.is_empty() {
                    self.confirm = Some(ConfirmKind::DeletePlaylists(names));
                }
            }
            KeyCode::Char('X') => {
                let names = self.selected_library_names();
                if !names.is_empty() {
                    self.confirm = Some(ConfirmKind::DeletePlaylistsAndTracks(names));
                }
            }
            _ => {}
        }
    }

    fn selected_library_names(&self) -> Vec<String> {
        let mut idxs: Vec<usize> = self.library_selected.iter().copied().collect();
        idxs.sort();
        idxs.iter()
            .filter_map(|&i| self.library.get(i).map(|lp| lp.name.clone()))
            .collect()
    }

    fn start_downloads(&mut self) {
        let selected: Vec<usize> = {
            let mut v: Vec<usize> = self.download_selected.iter().copied().collect();
            v.sort();
            v
        };
        for idx in selected {
            let p = match self.playlists.get(idx) {
                Some(p) => p.clone(),
                None => continue,
            };
            if matches!(
                self.statuses.get(&p.title),
                Some(DownloadStatus::Running { .. } | DownloadStatus::Queued)
            ) {
                continue;
            }
            self.statuses
                .insert(p.title.clone(), DownloadStatus::Queued);

            let sem = self.sem.clone();
            let app_tx = self.app_tx.clone();
            let cancel = self.cancel.clone();
            let cfg = self.config.clone();
            tokio::spawn(async move {
                let _permit = match sem.acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => return,
                };

                // Seed the download archive from the matching Music playlist (if any).
                let archive_seed = music::playlist_track_comments(&p.title)
                    .await
                    .unwrap_or_default();

                let (dtx, mut drx) = mpsc::unbounded_channel::<DownloadEvent>();
                let app_tx2 = app_tx.clone();
                let forwarder = tokio::spawn(async move {
                    while let Some(de) = drx.recv().await {
                        if app_tx2.send(AppEvent::Download(de)).is_err() {
                            break;
                        }
                    }
                });
                ytdlp::download_playlist(p.url, p.title, cfg, archive_seed, dtx, cancel).await;
                let _ = forwarder.await;
            });
        }
    }

    pub fn handle_app_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Download(de) => self.handle_download_event(de),
            AppEvent::LibraryRefreshed(lp) => {
                self.library = lp;
                if !self.library.is_empty() && self.library_cursor >= self.library.len() {
                    self.library_cursor = self.library.len() - 1;
                }
                self.library_selected.retain(|&i| i < self.library.len());
            }
            AppEvent::PlaylistsRefreshed(pl) => {
                self.playlists = pl;
                if !self.playlists.is_empty() && self.download_cursor >= self.playlists.len() {
                    self.download_cursor = self.playlists.len() - 1;
                }
                self.download_selected.retain(|&i| i < self.playlists.len());
                self.status_msg = Some(format!("refreshed: {} playlists", self.playlists.len()));
            }
            AppEvent::BackgroundError(msg) => {
                self.status_msg = Some(msg);
            }
        }
    }

    fn handle_download_event(&mut self, event: DownloadEvent) {
        match event {
            DownloadEvent::Started { playlist_title } => {
                self.statuses.insert(
                    playlist_title,
                    DownloadStatus::Running {
                        track_index: 0,
                        track_total: 0,
                        track_title: String::new(),
                        percent: 0.0,
                        speed: None,
                        eta: None,
                        imported_so_far: 0,
                    },
                );
            }
            DownloadEvent::Progress {
                playlist_title,
                track_index,
                track_total,
                track_title,
                percent,
                speed,
                eta,
            } => {
                let imported_so_far = match self.statuses.get(&playlist_title) {
                    Some(DownloadStatus::Running {
                        imported_so_far, ..
                    }) => *imported_so_far,
                    _ => 0,
                };
                self.statuses.insert(
                    playlist_title,
                    DownloadStatus::Running {
                        track_index,
                        track_total,
                        track_title,
                        percent,
                        speed,
                        eta,
                        imported_so_far,
                    },
                );
            }
            DownloadEvent::TrackImported { playlist_title, .. } => {
                if let Some(DownloadStatus::Running {
                    imported_so_far, ..
                }) = self.statuses.get_mut(&playlist_title)
                {
                    *imported_so_far += 1;
                }
            }
            DownloadEvent::TrackFailed { .. } => {}
            DownloadEvent::ImportDone {
                playlist_title,
                tracks_imported,
                tracks_expected,
            } => {
                self.statuses.insert(
                    playlist_title,
                    DownloadStatus::Done {
                        tracks_imported,
                        tracks_expected,
                    },
                );
                // refresh library so the new playlist shows up there
                let app_tx = self.app_tx.clone();
                tokio::spawn(async move {
                    if let Ok(lp) = music::list_user_playlists().await {
                        let _ = app_tx.send(AppEvent::LibraryRefreshed(lp));
                    }
                });
            }
            DownloadEvent::Failed {
                playlist_title,
                message,
            } => {
                self.statuses
                    .insert(playlist_title, DownloadStatus::Failed { message });
            }
        }
    }

    fn refresh(&mut self) {
        let url = self.channel_url.clone();
        let ytdlp_path = self.config.ytdlp.clone();
        let app_tx = self.app_tx.clone();
        tokio::spawn(async move {
            match ytdlp::list_playlists(&url, &ytdlp_path).await {
                Ok(p) => {
                    let _ = app_tx.send(AppEvent::PlaylistsRefreshed(p));
                }
                Err(e) => {
                    let _ = app_tx
                        .send(AppEvent::BackgroundError(format!("playlists refresh: {e}")));
                }
            }
        });
        let app_tx2 = self.app_tx.clone();
        tokio::spawn(async move {
            match music::list_user_playlists().await {
                Ok(p) => {
                    let _ = app_tx2.send(AppEvent::LibraryRefreshed(p));
                }
                Err(e) => {
                    let _ = app_tx2
                        .send(AppEvent::BackgroundError(format!("library refresh: {e}")));
                }
            }
        });
        self.status_msg = Some("refreshing...".to_string());
    }

    fn execute_delete(&mut self, names: Vec<String>, with_tracks: bool) {
        let app_tx = self.app_tx.clone();
        tokio::spawn(async move {
            let mut errors = Vec::new();
            for name in &names {
                let res = if with_tracks {
                    music::delete_playlist_and_tracks(name).await
                } else {
                    music::delete_user_playlist(name).await
                };
                if let Err(e) = res {
                    errors.push(format!("{name}: {e}"));
                }
            }
            match music::list_user_playlists().await {
                Ok(lp) => {
                    let _ = app_tx.send(AppEvent::LibraryRefreshed(lp));
                }
                Err(e) => {
                    let _ = app_tx.send(AppEvent::BackgroundError(e.to_string()));
                }
            }
            if !errors.is_empty() {
                let _ = app_tx.send(AppEvent::BackgroundError(format!(
                    "delete errors: {}",
                    errors.join("; ")
                )));
            }
        });
        self.library_selected.clear();
    }
}
