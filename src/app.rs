use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use egui::{Align, Color32, CornerRadius, FontId, Frame, Layout, RichText, Sense, Stroke};
use souvlaki::{
    MediaControlEvent, MediaControls, MediaMetadata, MediaPlayback, MediaPosition, PlatformConfig,
    SeekDirection,
};

use crate::audio::AudioEngine;
use crate::config::{Config, Dirs, ServerConfig};
use crate::db::Db;
use crate::model::{
    Album, AlbumDetail, AlbumOrigin, ArtistRow, Track, TrackSource, fmt_duration, server_album_key,
};
use crate::scan;
use crate::subsonic::{SubAlbum, SubsonicClient};

const ACCENT: Color32 = Color32::from_rgb(122, 162, 255);
const BG: Color32 = Color32::from_rgb(16, 16, 20);
const PANEL: Color32 = Color32::from_rgb(22, 22, 28);
const CARD: Color32 = Color32::from_rgb(32, 32, 40);
const MUTED: Color32 = Color32::from_rgb(150, 150, 162);

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Library,
    AlbumDetail,
    ArtistDetail,
    ServerAlbums,
    Search,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LibTab {
    Albums,
    Artists,
}

enum Event {
    ScanProgress(usize),
    ScanDone(usize),
    ScanError(String),
    Media(MediaCommand),
    ServerAlbums {
        server_id: i64,
        albums: Vec<Album>,
    },
    ServerAlbumDetail {
        server_id: i64,
        detail: AlbumDetail,
    },
    ServerError {
        server_id: i64,
        message: String,
    },
    CoverImage {
        key: String,
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    },
    CoverFailed(String),
    PlayBytes {
        seq: u64,
        bytes: Vec<u8>,
    },
    PlayFailed {
        seq: u64,
        message: String,
    },
    Info(String),
}

enum UiAction {
    PlayTracks(Vec<Track>, usize),
    PlayIndex(usize),
    Enqueue(Vec<Track>),
    OpenLocalAlbum(Album),
    OpenArtist(String),
    OpenServer(i64),
    OpenServerAlbum(Album),
    Toggle,
    Next,
    Prev,
    Seek(f64),
    Volume(f32),
    Scan,
}

#[derive(Clone, Copy, Debug)]
enum MediaCommand {
    Play,
    Pause,
    Toggle,
    Next,
    Previous,
    Stop,
    SeekBy(f64),
    SetPosition(f64),
    SetVolume(f32),
    Quit,
}

#[derive(Default)]
struct SettingsState {
    new_folder: String,
    server_name: String,
    server_url: String,
    server_user: String,
    server_password: String,
}

pub struct AuralApp {
    config: Config,
    config_path: PathBuf,
    cover_dir: PathBuf,
    db: Db,
    audio: Option<AudioEngine>,

    view: View,
    lib_tab: LibTab,
    albums: Vec<Album>,
    artists: Vec<ArtistRow>,
    album_detail: Option<AlbumDetail>,
    artist_detail: Option<(String, Vec<Track>)>,
    detail_loading: bool,
    track_count: u64,

    active_server: Option<i64>,
    server_albums: Vec<Album>,
    server_loading: bool,

    queue: Vec<Track>,
    queue_pos: usize,
    now_playing: Option<Track>,
    playing: bool,
    position: f64,
    volume: f32,
    seeking: bool,

    pending_seq: u64,
    pending_play: Option<u64>,

    controls: Option<MediaControls>,
    mpris_meta: Option<(String, String, String, u64, Option<String>)>,
    mpris_playing: Option<bool>,
    mpris_volume: f64,

    images: HashMap<String, egui::TextureHandle>,
    image_requests: HashSet<String>,
    image_failed: HashSet<String>,

    tx: Sender<Event>,
    rx: Receiver<Event>,

    status: String,
    status_at: Instant,
    scan_running: bool,

    search: String,
    search_results: Vec<Album>,
    show_settings: bool,
    show_queue: bool,
    settings: SettingsState,
}

impl AuralApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_style(&cc.egui_ctx);

        let dirs = Dirs::resolve();
        let config_path = dirs.config_file();
        let cover_dir = dirs.covers_dir();
        let config = Config::load(&config_path);
        let db = Db::open(&dirs.db_file()).expect("não foi possível abrir o banco da biblioteca");

        let (audio, audio_error) = match AudioEngine::new() {
            Ok(engine) => (Some(engine), None),
            Err(err) => (None, Some(format!("{err:#}"))),
        };
        if let Some(engine) = &audio {
            engine.set_volume(config.volume);
        }

        let albums = db.albums().unwrap_or_default();
        let artists = db.artists().unwrap_or_default();
        let track_count = db.track_count().unwrap_or(0);

        let (tx, rx) = channel();
        let controls = init_media_controls(&tx);

        let mut settings = SettingsState::default();
        if config.local_folders.is_empty()
            && let Some(music) = default_music_dir()
        {
            settings.new_folder = music.to_string_lossy().to_string();
        }

        let mut app = Self {
            volume: config.volume,
            config,
            config_path,
            cover_dir,
            db,
            audio,
            view: View::Library,
            lib_tab: LibTab::Albums,
            albums,
            artists,
            album_detail: None,
            artist_detail: None,
            detail_loading: false,
            track_count,
            active_server: None,
            server_albums: Vec::new(),
            server_loading: false,
            queue: Vec::new(),
            queue_pos: 0,
            now_playing: None,
            playing: false,
            position: 0.0,
            seeking: false,
            pending_seq: 0,
            pending_play: None,
            controls,
            mpris_meta: None,
            mpris_playing: None,
            mpris_volume: 0.0,
            images: HashMap::new(),
            image_requests: HashSet::new(),
            image_failed: HashSet::new(),
            tx,
            rx,
            status: String::new(),
            status_at: Instant::now(),
            scan_running: false,
            search: String::new(),
            search_results: Vec::new(),
            show_settings: false,
            show_queue: true,
            settings,
        };

        if let Some(err) = audio_error {
            app.set_status(format!("Áudio indisponível: {err}"));
        } else if app.albums.is_empty() {
            app.set_status("Bem-vindo! Configure uma pasta local ou um servidor.");
        } else {
            app.set_status(format!("{} faixas na biblioteca", app.track_count));
        }
        if app.albums.is_empty() && !app.config.local_folders.is_empty() {
            app.start_scan();
        }
        app
    }

    fn poll_events(&mut self, ctx: &egui::Context) {
        while let Ok(event) = self.rx.try_recv() {
            match event {
                Event::ScanProgress(n) => {
                    self.set_status(format!("Escaneando biblioteca… {n} faixas"));
                }
                Event::ScanDone(n) => {
                    self.scan_running = false;
                    self.reload_library();
                    self.set_status(format!("Biblioteca atualizada: {n} faixas"));
                }
                Event::ScanError(message) => {
                    self.scan_running = false;
                    self.set_status(format!("Falha no scan: {message}"));
                }
                Event::ServerAlbums { server_id, albums } => {
                    if self.active_server == Some(server_id) {
                        self.server_albums = albums;
                        self.server_loading = false;
                        self.view = View::ServerAlbums;
                        self.set_status("Álbuns do servidor carregados");
                    }
                }
                Event::ServerAlbumDetail { server_id, detail } => {
                    if self.active_server == Some(server_id) {
                        self.album_detail = Some(detail);
                        self.detail_loading = false;
                    }
                }
                Event::ServerError { server_id, message } => {
                    self.server_loading = false;
                    self.detail_loading = false;
                    if self.active_server == Some(server_id) {
                        self.set_status(format!("Servidor: {message}"));
                    }
                }
                Event::CoverImage {
                    key,
                    width,
                    height,
                    rgba,
                } => {
                    self.image_requests.remove(&key);
                    if width > 0 && height > 0 {
                        let image =
                            egui::ColorImage::from_rgba_unmultiplied([width, height], &rgba);
                        let texture =
                            ctx.load_texture(key.clone(), image, egui::TextureOptions::LINEAR);
                        self.images.insert(key, texture);
                    }
                }
                Event::CoverFailed(key) => {
                    self.image_requests.remove(&key);
                    self.image_failed.insert(key);
                }
                Event::PlayBytes { seq, bytes } => {
                    if self.pending_play == Some(seq) {
                        self.pending_play = None;
                        let result = self.audio.as_ref().map(|audio| audio.play_bytes(bytes));
                        match result {
                            Some(Ok(())) => {
                                self.playing = true;
                                self.position = 0.0;
                            }
                            Some(Err(err)) => {
                                self.playing = false;
                                self.set_status(format!("Falha ao tocar stream: {err:#}"));
                            }
                            None => {
                                self.playing = false;
                                self.set_status("Saída de áudio indisponível");
                            }
                        }
                    }
                }
                Event::PlayFailed { seq, message } => {
                    if self.pending_play == Some(seq) {
                        self.pending_play = None;
                        self.playing = false;
                        self.set_status(format!("Download falhou: {message}"));
                    }
                }
                Event::Info(message) => self.set_status(message),
                Event::Media(command) => match command {
                    MediaCommand::Play => self.resume_playback(),
                    MediaCommand::Pause => self.pause_playback(),
                    MediaCommand::Toggle => self.toggle_play(),
                    MediaCommand::Next => self.next_track(),
                    MediaCommand::Previous => self.prev_track(),
                    MediaCommand::Stop => self.stop_playback(),
                    MediaCommand::SeekBy(delta) => {
                        let duration = self
                            .now_playing
                            .as_ref()
                            .map(|track| track.duration_ms as f64 / 1000.0)
                            .unwrap_or(0.0);
                        let target = (self.position + delta).max(0.0);
                        let target = if duration > 0.0 {
                            target.min(duration)
                        } else {
                            target
                        };
                        self.apply(UiAction::Seek(target));
                    }
                    MediaCommand::SetPosition(position) => {
                        self.apply(UiAction::Seek(position.max(0.0)));
                    }
                    MediaCommand::SetVolume(volume) => {
                        self.apply(UiAction::Volume(volume.clamp(0.0, 1.0)));
                    }
                    MediaCommand::Quit => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                },
            }
        }
    }

    fn tick(&mut self, ctx: &egui::Context) {
        if let Some(audio) = &self.audio {
            if self.playing && !self.seeking {
                self.position = audio.position().as_secs_f64();
            }
            if self.playing && self.pending_play.is_none() && !audio.is_paused() && audio.is_empty()
            {
                if self.queue_pos + 1 < self.queue.len() {
                    self.play_index(self.queue_pos + 1);
                } else {
                    self.playing = false;
                    self.position = 0.0;
                }
            }
        }
        if !self.status.is_empty() && self.status_at.elapsed() > Duration::from_secs(9) {
            self.status.clear();
        }
        ctx.request_repaint_after(Duration::from_millis(300));
    }

    fn reload_library(&mut self) {
        match self.db.albums() {
            Ok(albums) => self.albums = albums,
            Err(err) => self.set_status(format!("Erro lendo álbuns: {err:#}")),
        }
        match self.db.artists() {
            Ok(artists) => self.artists = artists,
            Err(err) => self.set_status(format!("Erro lendo artistas: {err:#}")),
        }
        self.track_count = self.db.track_count().unwrap_or(0);
    }

    fn set_status(&mut self, message: impl Into<String>) {
        self.status = message.into();
        self.status_at = Instant::now();
    }

    fn sync_mpris(&mut self) {
        let meta = self.now_playing.as_ref().map(|track| {
            let cover = track.cover.as_deref().map(|cover| {
                if cover.starts_with("http://") || cover.starts_with("https://") {
                    cover.to_string()
                } else {
                    reqwest::Url::from_file_path(cover)
                        .map(|url| url.to_string())
                        .unwrap_or_else(|_| cover.to_string())
                }
            });
            (
                track.title.clone(),
                track.artist.clone(),
                track.album.clone(),
                track.duration_ms,
                cover,
            )
        });
        let playing = self.playing;
        let volume = self.volume as f64;
        let position = self.position;

        let Some(controls) = self.controls.as_mut() else {
            return;
        };

        if meta != self.mpris_meta {
            let metadata = match &meta {
                Some((title, artist, album, duration_ms, cover)) => MediaMetadata {
                    title: Some(title.as_str()),
                    artist: Some(artist.as_str()),
                    album: Some(album.as_str()),
                    cover_url: cover.as_deref(),
                    duration: if *duration_ms > 0 {
                        Some(Duration::from_millis(*duration_ms))
                    } else {
                        None
                    },
                },
                None => MediaMetadata::default(),
            };
            if let Err(err) = controls.set_metadata(metadata) {
                eprintln!("MPRIS metadata: {err}");
            }
            self.mpris_meta = meta;
        }

        if self.mpris_playing != Some(playing) {
            let progress = Some(MediaPosition(Duration::from_secs_f64(position.max(0.0))));
            let playback = if playing {
                MediaPlayback::Playing { progress }
            } else if self.now_playing.is_some() {
                MediaPlayback::Paused { progress }
            } else {
                MediaPlayback::Stopped
            };
            if let Err(err) = controls.set_playback(playback) {
                eprintln!("MPRIS playback: {err}");
            }
            self.mpris_playing = Some(playing);
        }

        if (self.mpris_volume - volume).abs() > 0.001 {
            if let Err(err) = controls.set_volume(volume) {
                eprintln!("MPRIS volume: {err}");
            }
            self.mpris_volume = volume;
        }
    }

    fn save_config(&mut self) {
        if let Err(err) = self.config.save(&self.config_path) {
            self.set_status(format!("Não foi possível salvar a configuração: {err:#}"));
        }
    }

    fn start_scan(&mut self) {
        if self.scan_running {
            return;
        }
        let folders: Vec<PathBuf> = self
            .config
            .local_folders
            .iter()
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
            .collect();
        if folders.is_empty() {
            self.set_status("Adicione uma pasta de músicas antes de escanear.");
            return;
        }
        self.scan_running = true;
        self.set_status("Escaneando biblioteca…");
        let db_path = self.db.path().to_path_buf();
        let cover_dir = self.cover_dir.clone();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let progress_tx = tx.clone();
            let result = scan::scan_library(&folders, &db_path, &cover_dir, move |count| {
                let _ = progress_tx.send(Event::ScanProgress(count));
            });
            match result {
                Ok(count) => {
                    let _ = tx.send(Event::ScanDone(count));
                }
                Err(err) => {
                    let _ = tx.send(Event::ScanError(format!("{err:#}")));
                }
            }
        });
    }

    fn on_search_changed(&mut self) {
        let query = self.search.trim().to_string();
        if query.is_empty() {
            self.search_results.clear();
            if self.view == View::Search {
                self.view = View::Library;
            }
            return;
        }
        match self.db.search_albums(&query) {
            Ok(results) => {
                self.search_results = results;
                self.view = View::Search;
            }
            Err(err) => self.set_status(format!("Busca falhou: {err:#}")),
        }
    }

    fn open_local_album(&mut self, album: &Album) {
        match self.db.album_tracks(&album.key) {
            Ok(tracks) => {
                self.album_detail = Some(AlbumDetail {
                    album: album.clone(),
                    tracks,
                });
                self.detail_loading = false;
                self.view = View::AlbumDetail;
            }
            Err(err) => self.set_status(format!("Erro abrindo álbum: {err:#}")),
        }
    }

    fn open_artist(&mut self, name: &str) {
        match self.db.artist_tracks(name) {
            Ok(tracks) => {
                self.artist_detail = Some((name.to_string(), tracks));
                self.view = View::ArtistDetail;
            }
            Err(err) => self.set_status(format!("Erro abrindo artista: {err:#}")),
        }
    }

    fn open_server(&mut self, server_id: i64) {
        let Some(cfg) = self
            .config
            .servers
            .iter()
            .find(|server| server.id == server_id)
            .cloned()
        else {
            self.set_status("Servidor não encontrado.");
            return;
        };
        self.active_server = Some(server_id);
        self.server_albums.clear();
        self.server_loading = true;
        self.view = View::ServerAlbums;
        self.set_status(format!("Carregando {}…", cfg.name));
        let tx = self.tx.clone();
        thread::spawn(move || {
            let client = match SubsonicClient::new(&cfg) {
                Ok(client) => client,
                Err(err) => {
                    let _ = tx.send(Event::ServerError {
                        server_id,
                        message: format!("{err:#}"),
                    });
                    return;
                }
            };
            match client.albums() {
                Ok(albums) => {
                    let albums = albums
                        .iter()
                        .map(|album| to_server_album(server_id, &client, album))
                        .collect();
                    let _ = tx.send(Event::ServerAlbums { server_id, albums });
                }
                Err(err) => {
                    let _ = tx.send(Event::ServerError {
                        server_id,
                        message: format!("{err:#}"),
                    });
                }
            }
        });
    }

    fn open_server_album(&mut self, server_id: i64, album_id: &str, fallback: Album) {
        let Some(cfg) = self
            .config
            .servers
            .iter()
            .find(|server| server.id == server_id)
            .cloned()
        else {
            self.set_status("Servidor não encontrado.");
            return;
        };
        self.view = View::AlbumDetail;
        self.detail_loading = true;
        self.album_detail = Some(AlbumDetail {
            album: fallback,
            tracks: Vec::new(),
        });
        let tx = self.tx.clone();
        let album_id = album_id.to_string();
        thread::spawn(move || {
            let client = match SubsonicClient::new(&cfg) {
                Ok(client) => client,
                Err(err) => {
                    let _ = tx.send(Event::ServerError {
                        server_id,
                        message: format!("{err:#}"),
                    });
                    return;
                }
            };
            match client.album(&album_id) {
                Ok(detail) => {
                    let cover = detail
                        .album
                        .cover_id
                        .as_ref()
                        .and_then(|id| client.cover_url(id, 400).ok());
                    let album = Album {
                        key: server_album_key(server_id, &detail.album.id),
                        title: detail.album.name.clone(),
                        artist: detail.album.artist.clone(),
                        year: detail.album.year,
                        cover: cover.clone(),
                        track_count: detail.songs.len() as u32,
                        duration_ms: detail.album.duration_ms,
                        origin: AlbumOrigin::Server {
                            server_id,
                            album_id: detail.album.id.clone(),
                        },
                    };
                    let tracks = detail
                        .songs
                        .iter()
                        .filter_map(|song| {
                            let url = client.stream_url(&song.id).ok()?;
                            Some(Track {
                                title: song.title.clone(),
                                artist: if song.artist.is_empty() {
                                    detail.album.artist.clone()
                                } else {
                                    song.artist.clone()
                                },
                                album: if song.album.is_empty() {
                                    detail.album.name.clone()
                                } else {
                                    song.album.clone()
                                },
                                duration_ms: song.duration_ms,
                                track_no: song.track,
                                disc_no: song.disc,
                                cover: cover.clone(),
                                source: TrackSource::Remote(url),
                            })
                        })
                        .collect();
                    let _ = tx.send(Event::ServerAlbumDetail {
                        server_id,
                        detail: AlbumDetail { album, tracks },
                    });
                }
                Err(err) => {
                    let _ = tx.send(Event::ServerError {
                        server_id,
                        message: format!("{err:#}"),
                    });
                }
            }
        });
    }

    fn play_index(&mut self, index: usize) {
        let Some(track) = self.queue.get(index).cloned() else {
            return;
        };
        self.queue_pos = index;
        self.position = 0.0;
        self.now_playing = Some(track.clone());
        self.seeking = false;
        match &track.source {
            TrackSource::Local(path) => {
                self.pending_play = None;
                let result = self.audio.as_ref().map(|audio| audio.play_path(path));
                match result {
                    Some(Ok(())) => {
                        self.playing = true;
                    }
                    Some(Err(err)) => {
                        self.playing = false;
                        self.set_status(format!("Falha ao tocar {}: {err:#}", track.title));
                    }
                    None => {
                        self.playing = false;
                        self.set_status("Saída de áudio indisponível");
                    }
                }
            }
            TrackSource::Remote(url) => {
                self.playing = true;
                self.pending_seq += 1;
                let seq = self.pending_seq;
                self.pending_play = Some(seq);
                self.set_status(format!("Baixando {}…", track.title));
                let url = url.clone();
                let tx = self.tx.clone();
                thread::spawn(move || match download_bytes(&url) {
                    Ok(bytes) => {
                        let _ = tx.send(Event::PlayBytes { seq, bytes });
                    }
                    Err(message) => {
                        let _ = tx.send(Event::PlayFailed { seq, message });
                    }
                });
            }
        }
    }

    fn resume_playback(&mut self) {
        if self.queue.is_empty() || self.now_playing.is_none() {
            if !self.queue.is_empty() {
                self.play_index(0);
            }
            return;
        }
        if self.pending_play.is_some() {
            return;
        }
        let Some(audio) = &self.audio else {
            self.set_status("Saída de áudio indisponível");
            return;
        };
        if audio.is_empty() {
            self.play_index(self.queue_pos.min(self.queue.len() - 1));
        } else {
            audio.resume();
            self.playing = true;
        }
    }

    fn pause_playback(&mut self) {
        if self.pending_play.take().is_some() {
            self.pending_seq += 1;
            self.playing = false;
            self.set_status("Download cancelado. Toque para baixar novamente.");
            return;
        }
        if let Some(audio) = &self.audio {
            audio.pause();
            self.playing = false;
        }
    }

    fn stop_playback(&mut self) {
        if let Some(audio) = &self.audio {
            audio.stop();
        }
        self.playing = false;
        self.position = 0.0;
        self.pending_play = None;
        self.now_playing = None;
    }

    fn toggle_play(&mut self) {
        if self.playing {
            self.pause_playback();
        } else {
            self.resume_playback();
        }
    }

    fn next_track(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        if self.queue_pos + 1 < self.queue.len() {
            self.play_index(self.queue_pos + 1);
        } else if let Some(audio) = &self.audio {
            audio.stop();
            self.playing = false;
            self.position = 0.0;
        }
    }

    fn prev_track(&mut self) {
        if self.queue.is_empty() {
            return;
        }
        if self.position > 3.0 {
            self.position = 0.0;
            if let Some(audio) = &self.audio {
                let _ = audio.seek(0.0);
            }
            return;
        }
        let index = self.queue_pos.saturating_sub(1);
        self.play_index(index);
    }

    fn apply(&mut self, action: UiAction) {
        match action {
            UiAction::PlayTracks(tracks, index) => {
                if tracks.is_empty() {
                    return;
                }
                self.queue = tracks;
                self.play_index(index.min(self.queue.len() - 1));
            }
            UiAction::PlayIndex(index) => self.play_index(index),
            UiAction::Enqueue(tracks) => {
                let was_empty = self.queue.is_empty();
                self.queue.extend(tracks);
                if was_empty {
                    self.play_index(0);
                }
            }
            UiAction::OpenLocalAlbum(album) => self.open_local_album(&album),
            UiAction::OpenArtist(name) => self.open_artist(&name),
            UiAction::OpenServer(id) => self.open_server(id),
            UiAction::OpenServerAlbum(album) => {
                if let AlbumOrigin::Server {
                    server_id,
                    album_id,
                } = album.origin.clone()
                {
                    self.open_server_album(server_id, &album_id, album);
                }
            }
            UiAction::Toggle => self.toggle_play(),
            UiAction::Next => self.next_track(),
            UiAction::Prev => self.prev_track(),
            UiAction::Seek(position) => {
                self.position = position;
                if let Some(audio) = &self.audio {
                    let _ = audio.seek(position);
                }
            }
            UiAction::Volume(volume) => {
                self.volume = volume;
                self.config.volume = volume;
                if let Some(audio) = &self.audio {
                    audio.set_volume(volume);
                }
            }
            UiAction::Scan => self.start_scan(),
        }
    }

    fn ensure_cover(&mut self, key: &str) {
        let key = key.trim();
        if key.is_empty()
            || self.images.contains_key(key)
            || self.image_requests.contains(key)
            || self.image_failed.contains(key)
        {
            return;
        }
        if self.image_requests.len() >= 12 {
            return;
        }
        self.image_requests.insert(key.to_string());
        let key_owned = key.to_string();
        let tx = self.tx.clone();
        if key.starts_with("http://") || key.starts_with("https://") {
            let url = key.to_string();
            thread::spawn(move || match download_bytes(&url) {
                Ok(bytes) => match decode_cover(&bytes) {
                    Some((width, height, rgba)) => {
                        let _ = tx.send(Event::CoverImage {
                            key: key_owned,
                            width,
                            height,
                            rgba,
                        });
                    }
                    None => {
                        let _ = tx.send(Event::CoverFailed(key_owned));
                    }
                },
                Err(message) => {
                    eprintln!("capa: {message}");
                    let _ = tx.send(Event::CoverFailed(key_owned));
                }
            });
        } else {
            let path = PathBuf::from(key);
            thread::spawn(move || match std::fs::read(&path) {
                Ok(bytes) => match decode_cover(&bytes) {
                    Some((width, height, rgba)) => {
                        let _ = tx.send(Event::CoverImage {
                            key: key_owned,
                            width,
                            height,
                            rgba,
                        });
                    }
                    None => {
                        let _ = tx.send(Event::CoverFailed(key_owned));
                    }
                },
                Err(_) => {
                    let _ = tx.send(Event::CoverFailed(key_owned));
                }
            });
        }
    }

    fn cover_keys_prepass(&mut self, keys: impl Iterator<Item = String>) {
        for key in keys {
            self.ensure_cover(&key);
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("topbar")
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(14, 8)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Aural").size(22.0).strong().color(ACCENT));
                    ui.label(
                        RichText::new("player de música para Linux")
                            .size(11.0)
                            .color(MUTED),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Configurações").clicked() {
                            self.show_settings = true;
                        }
                        if ui.button("Fila").clicked() {
                            self.show_queue = !self.show_queue;
                        }
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut self.search)
                                .hint_text("Buscar álbuns e artistas")
                                .desired_width(260.0),
                        );
                        if response.changed() {
                            self.on_search_changed();
                        }
                    });
                });
                if !self.status.is_empty() {
                    ui.label(RichText::new(&self.status).size(11.0).color(MUTED));
                }
            });
    }

    fn nav_panel(&mut self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        egui::Panel::left("nav")
            .resizable(false)
            .default_size(210.0)
            .frame(
                Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(10, 10)),
            )
            .show(ui, |ui| {
                ui.label(RichText::new("BIBLIOTECA").size(10.0).color(MUTED));
                ui.add_space(2.0);
                let albums_selected = self.view == View::Library && self.lib_tab == LibTab::Albums;
                if ui
                    .add_sized(
                        [ui.available_width(), 30.0],
                        egui::Button::new(RichText::new("Álbuns").size(14.0))
                            .selected(albums_selected),
                    )
                    .clicked()
                {
                    self.lib_tab = LibTab::Albums;
                    self.view = View::Library;
                }
                let artists_selected =
                    self.view == View::Library && self.lib_tab == LibTab::Artists;
                if ui
                    .add_sized(
                        [ui.available_width(), 30.0],
                        egui::Button::new(RichText::new("Artistas").size(14.0))
                            .selected(artists_selected),
                    )
                    .clicked()
                {
                    self.lib_tab = LibTab::Artists;
                    self.view = View::Library;
                }

                ui.add_space(12.0);
                ui.separator();
                ui.add_space(6.0);
                ui.label(RichText::new("SERVIDORES").size(10.0).color(MUTED));
                ui.add_space(2.0);

                if self.config.servers.is_empty() {
                    ui.label(
                        RichText::new("Nenhum servidor configurado")
                            .size(11.0)
                            .color(MUTED),
                    );
                } else {
                    for server in self.config.servers.clone() {
                        let selected = self.active_server == Some(server.id);
                        if ui
                            .add_sized(
                                [ui.available_width(), 28.0],
                                egui::Button::new(RichText::new(&server.name).size(13.0))
                                    .selected(selected),
                            )
                            .on_hover_text(&server.url)
                            .clicked()
                        {
                            actions.push(UiAction::OpenServer(server.id));
                        }
                    }
                }

                ui.add_space(8.0);
                if ui.button("+ Adicionar servidor").clicked() {
                    self.show_settings = true;
                }

                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    ui.label(
                        RichText::new(format!("{} faixas locais", self.track_count))
                            .size(11.0)
                            .color(MUTED),
                    );
                    ui.label(RichText::new("Aural 0.1").size(11.0).color(MUTED));
                });
            });
    }

    fn queue_panel(&mut self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        if !self.show_queue {
            return;
        }
        egui::Panel::right("queue")
            .resizable(true)
            .default_size(290.0)
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(10, 10)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(RichText::new("Fila").size(16.0));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("Limpar").clicked() {
                            self.queue.clear();
                            self.queue_pos = 0;
                            self.now_playing = None;
                            self.playing = false;
                            self.position = 0.0;
                            self.pending_play = None;
                            if let Some(audio) = &self.audio {
                                audio.stop();
                            }
                        }
                    });
                });
                ui.separator();
                if self.queue.is_empty() {
                    ui.label(RichText::new("A fila está vazia").size(12.0).color(MUTED));
                    return;
                }
                let queue = self.queue.clone();
                let current = self.queue_pos;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (index, track) in queue.iter().enumerate() {
                            let selected = index == current;
                            let label = format!("{:02}  {}", index + 1, track.title);
                            if ui
                                .selectable_label(selected, RichText::new(label).size(13.0))
                                .clicked()
                            {
                                actions.push(UiAction::PlayIndex(index));
                            }
                            ui.label(RichText::new(track.subtitle()).size(10.0).color(MUTED));
                        }
                    });
            });
    }

    fn player_bar(&mut self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        let mut volume = self.volume;
        let old_volume = self.volume;
        let mut seek_to = None;
        let mut toggle = false;
        let mut next = false;
        let mut previous = false;
        egui::Panel::bottom("player")
            .exact_size(104.0)
            .frame(
                Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(14, 10)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let track = self.now_playing.clone();
                    let cover_key = track.as_ref().and_then(|t| t.cover.clone());
                    cover_widget(ui, cover_key.as_deref(), &self.images, 64.0, 6, false);
                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        let title = track
                            .as_ref()
                            .map(|t| t.title.clone())
                            .unwrap_or_else(|| "Nada tocando".to_string());
                        ui.label(RichText::new(title).strong().size(15.0));
                        let subtitle = track
                            .as_ref()
                            .map(|t| t.subtitle())
                            .unwrap_or_else(|| "Escolha um álbum para começar".to_string());
                        ui.label(RichText::new(subtitle).size(12.0).color(MUTED));

                        let duration_ms = track.as_ref().map(|t| t.duration_ms).unwrap_or(0);
                        let mut duration = duration_ms as f64 / 1000.0;
                        if duration <= 0.0 {
                            duration = 1.0;
                        }
                        let mut position = self.position.min(duration);
                        ui.horizontal(|ui| {
                            if transport_button(ui, TransportIcon::Prev, false)
                                .on_hover_text("Anterior")
                                .clicked()
                            {
                                previous = true;
                            }
                            let icon = if self.playing {
                                TransportIcon::Pause
                            } else {
                                TransportIcon::Play
                            };
                            let label = if self.playing { "Pausar" } else { "Tocar" };
                            if transport_button(ui, icon, true)
                                .on_hover_text(label)
                                .clicked()
                            {
                                toggle = true;
                            }
                            if transport_button(ui, TransportIcon::Next, false)
                                .on_hover_text("Próxima")
                                .clicked()
                            {
                                next = true;
                            }
                            ui.label(
                                RichText::new(fmt_duration((position * 1000.0) as u64))
                                    .size(11.0)
                                    .color(MUTED),
                            );
                            let available = (ui.available_width() - 110.0).max(80.0);
                            ui.style_mut().spacing.slider_width = available;
                            let response = ui.add(
                                egui::Slider::new(&mut position, 0.0..=duration).show_value(false),
                            );
                            if response.drag_started() {
                                self.seeking = true;
                            }
                            if response.dragged() {
                                self.position = position;
                            } else if response.drag_stopped() {
                                self.seeking = false;
                                seek_to = Some(position);
                            } else if response.changed() {
                                seek_to = Some(position);
                            }
                            ui.label(
                                RichText::new(fmt_duration(duration_ms))
                                    .size(11.0)
                                    .color(MUTED),
                            );
                        });
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.style_mut().spacing.slider_width = 90.0;
                        ui.add(egui::Slider::new(&mut volume, 0.0..=1.0).show_value(false));
                        ui.label(RichText::new("Vol.").size(11.0).color(MUTED));
                    });
                });
            });

        if toggle {
            actions.push(UiAction::Toggle);
        }
        if next {
            actions.push(UiAction::Next);
        }
        if previous {
            actions.push(UiAction::Prev);
        }
        if let Some(position) = seek_to {
            actions.push(UiAction::Seek(position));
        }
        if (volume - old_volume).abs() > 0.0001 {
            actions.push(UiAction::Volume(volume));
        }
    }

    fn central_panel(&mut self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        egui::CentralPanel::default_margins()
            .frame(
                Frame::new()
                    .fill(BG)
                    .inner_margin(egui::Margin::symmetric(16, 14)),
            )
            .show(ui, |ui| match self.view {
                View::Library => self.library_view(ui, actions),
                View::AlbumDetail => self.album_detail_view(ui, actions),
                View::ArtistDetail => self.artist_detail_view(ui, actions),
                View::ServerAlbums => self.server_albums_view(ui, actions),
                View::Search => self.search_view(ui, actions),
            });
    }

    fn library_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        if self.albums.is_empty() {
            self.welcome_view(ui, actions);
            return;
        }
        match self.lib_tab {
            LibTab::Albums => {
                ui.horizontal(|ui| {
                    ui.heading(RichText::new("Álbuns").size(22.0));
                    ui.label(
                        RichText::new(format!("{} álbuns", self.albums.len()))
                            .size(12.0)
                            .color(MUTED),
                    );
                });
                ui.add_space(8.0);
                let keys: Vec<String> = self
                    .albums
                    .iter()
                    .filter_map(|album| album.cover.clone())
                    .collect();
                self.cover_keys_prepass(keys.into_iter());
                let clicked = {
                    let albums = &self.albums;
                    let images = &self.images;
                    album_grid(ui, albums, images, 150.0)
                };
                if let Some(index) = clicked {
                    let album = self.albums[index].clone();
                    actions.push(UiAction::OpenLocalAlbum(album));
                }
            }
            LibTab::Artists => {
                ui.horizontal(|ui| {
                    ui.heading(RichText::new("Artistas").size(22.0));
                    ui.label(
                        RichText::new(format!("{} artistas", self.artists.len()))
                            .size(12.0)
                            .color(MUTED),
                    );
                });
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for artist in &self.artists {
                            let text = format!(
                                "{}    ·    {} álbuns    ·    {} faixas",
                                artist.name, artist.album_count, artist.track_count
                            );
                            if ui
                                .selectable_label(false, RichText::new(text).size(14.0))
                                .clicked()
                            {
                                actions.push(UiAction::OpenArtist(artist.name.clone()));
                            }
                        }
                    });
            }
        }
    }

    fn album_detail_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        let Some(detail) = self.album_detail.clone() else {
            self.view = View::Library;
            return;
        };
        if ui.button("< Voltar").clicked() {
            self.view = if detail.album.is_server() {
                View::ServerAlbums
            } else {
                View::Library
            };
        }
        ui.add_space(6.0);
        if let Some(cover) = detail.album.cover.clone() {
            self.ensure_cover(&cover);
        }
        ui.horizontal(|ui| {
            let cover_key = detail.album.cover.as_deref();
            cover_widget(ui, cover_key, &self.images, 170.0, 10, false);
            ui.add_space(14.0);
            ui.vertical(|ui| {
                ui.heading(RichText::new(&detail.album.title).size(24.0));
                let mut subtitle = detail.album.artist.clone();
                if let Some(year) = detail.album.year {
                    subtitle.push_str(&format!("  ·  {year}"));
                }
                ui.label(RichText::new(subtitle).size(13.0).color(MUTED));
                ui.label(
                    RichText::new(format!(
                        "{} faixas  ·  {}",
                        detail.tracks.len(),
                        fmt_duration(detail.album.duration_ms)
                    ))
                    .size(12.0)
                    .color(MUTED),
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Tocar tudo").clicked() {
                        actions.push(UiAction::PlayTracks(detail.tracks.clone(), 0));
                    }
                    if ui.button("Adicionar à fila").clicked() {
                        actions.push(UiAction::Enqueue(detail.tracks.clone()));
                    }
                });
            });
        });
        ui.add_space(8.0);
        ui.separator();
        if detail.tracks.is_empty() {
            if self.detail_loading {
                ui.add_space(20.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Carregando faixas…").color(MUTED));
                });
            } else {
                ui.label(RichText::new("Nenhuma faixa encontrada.").color(MUTED));
            }
            return;
        }
        let mut play = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("track_grid")
                    .num_columns(3)
                    .striped(true)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        for (index, track) in detail.tracks.iter().enumerate() {
                            let disc = track.disc_no.unwrap_or(1);
                            let number = match (disc, track.track_no) {
                                (disc, Some(track_no)) if disc > 1 => {
                                    format!("{disc}.{track_no:02}")
                                }
                                (_, Some(track_no)) => format!("{track_no:02}"),
                                (disc, None) if disc > 1 => {
                                    format!("{disc}.{:02}", index + 1)
                                }
                                _ => format!("{:02}", index + 1),
                            };
                            ui.label(RichText::new(number).size(12.0).color(MUTED));
                            let response = ui.add(
                                egui::Label::new(RichText::new(&track.title).size(14.0))
                                    .sense(Sense::click()),
                            );
                            if response.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if response.clicked() {
                                play = Some(index);
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(
                                    RichText::new(fmt_duration(track.duration_ms))
                                        .size(12.0)
                                        .color(MUTED),
                                );
                            });
                            ui.end_row();
                        }
                    });
            });
        if let Some(index) = play {
            actions.push(UiAction::PlayTracks(detail.tracks.clone(), index));
        }
    }

    fn artist_detail_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        let Some((name, tracks)) = self.artist_detail.clone() else {
            self.view = View::Library;
            return;
        };
        if ui.button("< Voltar").clicked() {
            self.view = View::Library;
            self.lib_tab = LibTab::Artists;
        }
        ui.add_space(4.0);
        ui.heading(RichText::new(&name).size(24.0));
        ui.label(
            RichText::new(format!("{} faixas", tracks.len()))
                .size(12.0)
                .color(MUTED),
        );
        ui.add_space(6.0);
        if ui.button("Tocar tudo").clicked() {
            actions.push(UiAction::PlayTracks(tracks.clone(), 0));
        }
        ui.add_space(6.0);
        ui.separator();
        let mut play = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                egui::Grid::new("artist_track_grid")
                    .num_columns(3)
                    .striped(true)
                    .spacing([16.0, 6.0])
                    .show(ui, |ui| {
                        for (index, track) in tracks.iter().enumerate() {
                            ui.label(RichText::new(&track.album).size(11.0).color(MUTED));
                            let response = ui.add(
                                egui::Label::new(RichText::new(&track.title).size(14.0))
                                    .sense(Sense::click()),
                            );
                            if response.clicked() {
                                play = Some(index);
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(
                                    RichText::new(fmt_duration(track.duration_ms))
                                        .size(12.0)
                                        .color(MUTED),
                                );
                            });
                            ui.end_row();
                        }
                    });
            });
        if let Some(index) = play {
            actions.push(UiAction::PlayTracks(tracks.clone(), index));
        }
    }

    fn server_albums_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        let server_name = self
            .active_server
            .and_then(|id| {
                self.config
                    .servers
                    .iter()
                    .find(|server| server.id == id)
                    .map(|server| server.name.clone())
            })
            .unwrap_or_else(|| "Servidor".to_string());
        ui.horizontal(|ui| {
            ui.heading(RichText::new(&server_name).size(22.0));
            if self.server_loading {
                ui.spinner();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Atualizar").clicked()
                    && let Some(id) = self.active_server
                {
                    actions.push(UiAction::OpenServer(id));
                }
            });
        });
        ui.add_space(8.0);
        if self.server_loading && self.server_albums.is_empty() {
            ui.label(RichText::new("Carregando álbuns…").color(MUTED));
            return;
        }
        if self.server_albums.is_empty() {
            ui.label(RichText::new("Nenhum álbum retornado pelo servidor.").color(MUTED));
            return;
        }
        let keys: Vec<String> = self
            .server_albums
            .iter()
            .filter_map(|album| album.cover.clone())
            .collect();
        self.cover_keys_prepass(keys.into_iter());
        let clicked = {
            let albums = &self.server_albums;
            let images = &self.images;
            album_grid(ui, albums, images, 150.0)
        };
        if let Some(index) = clicked {
            let album = self.server_albums[index].clone();
            actions.push(UiAction::OpenServerAlbum(album));
        }
    }

    fn search_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        ui.heading(RichText::new(format!("Busca: {}", self.search.trim())).size(22.0));
        ui.label(
            RichText::new(format!("{} resultados", self.search_results.len()))
                .size(12.0)
                .color(MUTED),
        );
        ui.add_space(8.0);
        if self.search_results.is_empty() {
            ui.label(RichText::new("Nada encontrado.").color(MUTED));
            return;
        }
        let keys: Vec<String> = self
            .search_results
            .iter()
            .filter_map(|album| album.cover.clone())
            .collect();
        self.cover_keys_prepass(keys.into_iter());
        let clicked = {
            let albums = &self.search_results;
            let images = &self.images;
            album_grid(ui, albums, images, 150.0)
        };
        if let Some(index) = clicked {
            let album = self.search_results[index].clone();
            actions.push(UiAction::OpenLocalAlbum(album));
        }
    }

    fn welcome_view(&mut self, ui: &mut egui::Ui, actions: &mut Vec<UiAction>) {
        ui.vertical_centered(|ui| {
            ui.add_space(70.0);
            let (logo_rect, _) =
                ui.allocate_exact_size(egui::vec2(92.0, 92.0), Sense::hover());
            paint_vinyl(ui, logo_rect);
            ui.add_space(8.0);
            ui.heading(RichText::new("Bem-vindo ao Aural").size(28.0));
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Player nativo para Linux com biblioteca local e servidores OpenSubsonic/Navidrome.",
                )
                .size(13.0)
                .color(MUTED),
            );
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                if ui.button("Escanear pasta de música").clicked() {
                    if self.config.local_folders.is_empty()
                        && let Some(music) = default_music_dir()
                    {
                        self.config
                            .local_folders
                            .push(music.to_string_lossy().to_string());
                        self.save_config();
                    }
                    actions.push(UiAction::Scan);
                }
                if ui.button("Configurar servidor").clicked() {
                    self.show_settings = true;
                }
            });
            ui.add_space(8.0);
            ui.label(
                RichText::new("Dica: configure pastas e servidores em “Configurações”.")
                    .size(11.0)
                    .color(MUTED),
            );
        });
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.show_settings {
            return;
        }
        let mut open = true;
        egui::Window::new("Configurações")
            .open(&mut open)
            .default_width(560.0)
            .resizable(true)
            .show(ctx, |ui| {
                self.settings_body(ui);
            });
        self.show_settings = open;
    }

    fn settings_body(&mut self, ui: &mut egui::Ui) {
        ui.heading(RichText::new("Biblioteca local").size(16.0));
        ui.add_space(4.0);
        let mut remove_folder = None;
        for (index, folder) in self.config.local_folders.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(folder).size(12.0));
                if ui.small_button("remover").clicked() {
                    remove_folder = Some(index);
                }
            });
        }
        if let Some(index) = remove_folder {
            self.config.local_folders.remove(index);
            self.save_config();
        }
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.new_folder)
                    .hint_text("/home/você/Música")
                    .desired_width(330.0),
            );
            if ui.button("Adicionar").clicked() {
                let folder = self.settings.new_folder.trim().to_string();
                if !folder.is_empty() {
                    self.config.local_folders.push(folder);
                    self.settings.new_folder.clear();
                    self.save_config();
                }
            }
            if ui.button("Usar ~/Music").clicked()
                && let Some(music) = default_music_dir()
            {
                self.settings.new_folder = music.to_string_lossy().to_string();
            }
        });
        ui.horizontal(|ui| {
            let label = if self.scan_running {
                "Escaneando…"
            } else {
                "Escanear agora"
            };
            if ui
                .add_enabled(!self.scan_running, egui::Button::new(label))
                .clicked()
            {
                self.start_scan();
            }
            if self.scan_running {
                ui.spinner();
            }
        });

        ui.add_space(10.0);
        ui.separator();
        ui.heading(RichText::new("Servidores Subsonic / OpenSubsonic").size(16.0));
        ui.add_space(4.0);
        let mut remove_server = None;
        let mut test_server = None;
        for (index, server) in self.config.servers.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&server.name).strong().size(13.0));
                ui.label(RichText::new(&server.url).size(11.0).color(MUTED));
                if ui.small_button("testar").clicked() {
                    test_server = Some(server.clone());
                }
                if ui.small_button("remover").clicked() {
                    remove_server = Some(index);
                }
            });
        }
        if let Some(index) = remove_server {
            let removed = self.config.servers.remove(index);
            if self.active_server == Some(removed.id) {
                self.active_server = None;
                self.server_albums.clear();
            }
            self.save_config();
        }
        if let Some(server) = test_server {
            let tx = self.tx.clone();
            let name = server.name.clone();
            thread::spawn(move || {
                let message = match SubsonicClient::new(&server).and_then(|client| client.ping()) {
                    Ok(()) => format!("{name}: conexão OK"),
                    Err(err) => format!("{name}: {err:#}"),
                };
                let _ = tx.send(Event::Info(message));
            });
        }

        ui.add_space(6.0);
        egui::Grid::new("server_form")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.label("Nome");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.server_name).desired_width(320.0),
                );
                ui.end_row();
                ui.label("URL");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.server_url)
                        .hint_text("https://navidrome.exemplo.com")
                        .desired_width(320.0),
                );
                ui.end_row();
                ui.label("Usuário");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.server_user).desired_width(320.0),
                );
                ui.end_row();
                ui.label("Senha");
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.server_password)
                        .password(true)
                        .desired_width(320.0),
                );
                ui.end_row();
            });
        if ui.button("Adicionar servidor").clicked() {
            let name = self.settings.server_name.trim().to_string();
            let url = self.settings.server_url.trim().to_string();
            if name.is_empty() || url.is_empty() {
                self.set_status("Informe nome e URL do servidor.");
            } else {
                let id = self.config.next_server_id();
                self.config.servers.push(ServerConfig {
                    id,
                    name,
                    url,
                    username: self.settings.server_user.clone(),
                    password: self.settings.server_password.clone(),
                });
                self.settings.server_name.clear();
                self.settings.server_url.clear();
                self.settings.server_user.clear();
                self.settings.server_password.clear();
                self.save_config();
                self.set_status("Servidor adicionado.");
            }
        }
        ui.add_space(10.0);
        ui.separator();
        ui.label(
            RichText::new("As credenciais ficam em ~/.config/aural/config.toml (permissão 600).")
                .size(10.0)
                .color(MUTED),
        );
    }
}

impl eframe::App for AuralApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_events(ctx);
        self.tick(ctx);
        self.sync_mpris();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if let Some(cover) = self.now_playing.as_ref().and_then(|t| t.cover.clone()) {
            self.ensure_cover(&cover);
        }

        let mut actions = Vec::new();
        self.top_bar(ui);
        self.player_bar(ui, &mut actions);
        self.nav_panel(ui, &mut actions);
        self.queue_panel(ui, &mut actions);
        self.central_panel(ui, &mut actions);
        self.settings_window(&ctx);

        for action in actions {
            self.apply(action);
        }
        self.sync_mpris();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let _ = self.config.save(&self.config_path);
    }
}

fn configure_style(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = PANEL;
    visuals.extreme_bg_color = Color32::from_rgb(10, 10, 13);
    visuals.faint_bg_color = PANEL;
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.45);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.noninteractive.bg_fill = PANEL;
    visuals.widgets.inactive.bg_fill = CARD;
    visuals.widgets.inactive.weak_bg_fill = CARD;
    visuals.widgets.active.bg_fill = ACCENT.gamma_multiply(0.35);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(52, 52, 64);
    ctx.set_visuals_of(egui::Theme::Dark, visuals);

    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style
        .text_styles
        .insert(egui::TextStyle::Heading, FontId::proportional(22.0));
    style
        .text_styles
        .insert(egui::TextStyle::Body, FontId::proportional(13.0));
    ctx.set_style_of(egui::Theme::Dark, style);
}

fn album_grid(
    ui: &mut egui::Ui,
    albums: &[Album],
    images: &HashMap<String, egui::TextureHandle>,
    card_size: f32,
) -> Option<usize> {
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (index, album) in albums.iter().enumerate() {
                    if album_card(ui, album, images, card_size) {
                        clicked = Some(index);
                    }
                    ui.add_space(4.0);
                }
            });
        });
    clicked
}

fn album_card(
    ui: &mut egui::Ui,
    album: &Album,
    images: &HashMap<String, egui::TextureHandle>,
    size: f32,
) -> bool {
    let mut clicked = false;
    ui.allocate_ui_with_layout(
        egui::vec2(size, size + 52.0),
        Layout::top_down(Align::Center),
        |ui| {
            ui.set_width(size);
            let texture = album.cover.as_deref().and_then(|key| images.get(key));
            let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), Sense::click());
            paint_cover(ui, rect, texture, 8);
            if response.hovered() {
                ui.painter().rect_stroke(
                    rect,
                    CornerRadius::same(8),
                    Stroke::new(1.5, ACCENT),
                    egui::StrokeKind::Inside,
                );
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.clicked() {
                clicked = true;
            }
            let title = ui
                .add(egui::Label::new(RichText::new(&album.title).strong().size(13.0)).truncate());
            if title.clicked() {
                clicked = true;
            }
            let subtitle = match album.year {
                Some(year) => format!("{} · {year}", album.artist),
                None if album.track_count > 0 => {
                    format!("{} · {} faixas", album.artist, album.track_count)
                }
                None => album.artist.clone(),
            };
            ui.add(egui::Label::new(RichText::new(subtitle).size(11.0).color(MUTED)).truncate());
        },
    );
    clicked
}

fn cover_widget(
    ui: &mut egui::Ui,
    key: Option<&str>,
    images: &HashMap<String, egui::TextureHandle>,
    size: f32,
    rounding: u8,
    clickable: bool,
) -> egui::Response {
    let sense = if clickable {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), sense);
    let texture = key.and_then(|key| images.get(key));
    paint_cover(ui, rect, texture, rounding);
    response
}

fn paint_cover(
    ui: &egui::Ui,
    rect: egui::Rect,
    texture: Option<&egui::TextureHandle>,
    rounding: u8,
) {
    if let Some(texture) = texture {
        ui.painter().image(
            texture.id(),
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(rounding),
            Color32::from_rgb(26, 26, 34),
        );
        let inner = rect.shrink(rect.width() * 0.06);
        paint_vinyl(ui, inner);
    }
}

#[derive(Clone, Copy)]
enum TransportIcon {
    Prev,
    Play,
    Pause,
    Next,
}

fn transport_button(ui: &mut egui::Ui, icon: TransportIcon, primary: bool) -> egui::Response {
    let size = if primary { 38.0 } else { 30.0 };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), Sense::click());
    let hovered = response.hovered();
    let background = if primary {
        if hovered {
            ACCENT
        } else {
            ACCENT.gamma_multiply(0.85)
        }
    } else if hovered {
        Color32::from_rgb(46, 46, 56)
    } else {
        Color32::TRANSPARENT
    };
    if background != Color32::TRANSPARENT {
        ui.painter()
            .circle_filled(rect.center(), size * 0.5, background);
    }
    let color = if primary {
        Color32::from_rgb(12, 12, 18)
    } else {
        Color32::from_rgb(205, 205, 218)
    };
    let center = rect.center();
    let stroke = Stroke::NONE;
    match icon {
        TransportIcon::Play => {
            let points = vec![
                center + egui::vec2(-4.0, -7.0),
                center + egui::vec2(-4.0, 7.0),
                center + egui::vec2(7.0, 0.0),
            ];
            ui.painter()
                .add(egui::Shape::convex_polygon(points, color, stroke));
        }
        TransportIcon::Pause => {
            let bar = egui::vec2(3.5, 13.0);
            ui.painter().rect_filled(
                egui::Rect::from_center_size(center + egui::vec2(-3.5, 0.0), bar),
                1,
                color,
            );
            ui.painter().rect_filled(
                egui::Rect::from_center_size(center + egui::vec2(3.5, 0.0), bar),
                1,
                color,
            );
        }
        TransportIcon::Prev => {
            let points = vec![
                center + egui::vec2(5.0, -6.5),
                center + egui::vec2(5.0, 6.5),
                center + egui::vec2(-4.0, 0.0),
            ];
            ui.painter()
                .add(egui::Shape::convex_polygon(points, color, stroke));
            ui.painter().rect_filled(
                egui::Rect::from_center_size(center + egui::vec2(-7.0, 0.0), egui::vec2(2.0, 13.0)),
                1,
                color,
            );
        }
        TransportIcon::Next => {
            let points = vec![
                center + egui::vec2(-5.0, -6.5),
                center + egui::vec2(-5.0, 6.5),
                center + egui::vec2(4.0, 0.0),
            ];
            ui.painter()
                .add(egui::Shape::convex_polygon(points, color, stroke));
            ui.painter().rect_filled(
                egui::Rect::from_center_size(center + egui::vec2(7.0, 0.0), egui::vec2(2.0, 13.0)),
                1,
                color,
            );
        }
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn paint_vinyl(ui: &egui::Ui, rect: egui::Rect) {
    let center = rect.center();
    let radius = rect.width().min(rect.height()) * 0.5;
    if radius <= 0.0 {
        return;
    }
    let painter = ui.painter();
    painter.circle_filled(center, radius, Color32::from_rgb(18, 18, 24));
    painter.circle_stroke(
        center,
        radius - 1.0,
        Stroke::new(1.0, Color32::from_rgb(64, 64, 80)),
    );
    for step in 1..=3 {
        painter.circle_stroke(
            center,
            radius * (0.45 + 0.11 * step as f32),
            Stroke::new(1.0, Color32::from_rgb(43, 43, 55)),
        );
    }
    painter.circle_filled(center, radius * 0.27, ACCENT);
    painter.circle_filled(center, radius * 0.055, BG);
}

fn to_server_album(server_id: i64, client: &SubsonicClient, album: &SubAlbum) -> Album {
    Album {
        key: server_album_key(server_id, &album.id),
        title: album.name.clone(),
        artist: album.artist.clone(),
        year: album.year,
        cover: album
            .cover_id
            .as_ref()
            .and_then(|id| client.cover_url(id, 400).ok()),
        track_count: album.song_count,
        duration_ms: album.duration_ms,
        origin: AlbumOrigin::Server {
            server_id,
            album_id: album.id.clone(),
        },
    }
}

fn default_music_dir() -> Option<PathBuf> {
    let music = dirs::home_dir()?.join("Music");
    music.is_dir().then_some(music)
}

fn init_media_controls(tx: &Sender<Event>) -> Option<MediaControls> {
    let config = PlatformConfig {
        display_name: "Aural",
        dbus_name: "org.aural.Aural",
        hwnd: None,
    };
    let mut controls = match MediaControls::new(config) {
        Ok(controls) => controls,
        Err(err) => {
            eprintln!("MPRIS indisponível: {err}");
            return None;
        }
    };
    let tx = tx.clone();
    let result = controls.attach(move |event| {
        let command = match event {
            MediaControlEvent::Play => Some(MediaCommand::Play),
            MediaControlEvent::Pause => Some(MediaCommand::Pause),
            MediaControlEvent::Toggle => Some(MediaCommand::Toggle),
            MediaControlEvent::Next => Some(MediaCommand::Next),
            MediaControlEvent::Previous => Some(MediaCommand::Previous),
            MediaControlEvent::Stop => Some(MediaCommand::Stop),
            MediaControlEvent::Seek(direction) => Some(MediaCommand::SeekBy(match direction {
                SeekDirection::Forward => 10.0,
                SeekDirection::Backward => -10.0,
            })),
            MediaControlEvent::SeekBy(direction, duration) => {
                let seconds = duration.as_secs_f64();
                Some(MediaCommand::SeekBy(match direction {
                    SeekDirection::Forward => seconds,
                    SeekDirection::Backward => -seconds,
                }))
            }
            MediaControlEvent::SetPosition(MediaPosition(position)) => {
                Some(MediaCommand::SetPosition(position.as_secs_f64()))
            }
            MediaControlEvent::SetVolume(volume) => Some(MediaCommand::SetVolume(volume as f32)),
            MediaControlEvent::Quit => Some(MediaCommand::Quit),
            _ => None,
        };
        if let Some(command) = command {
            let _ = tx.send(Event::Media(command));
        }
    });
    match result {
        Ok(()) => Some(controls),
        Err(err) => {
            eprintln!("MPRIS: {err}");
            None
        }
    }
}

fn download_bytes(url: &str) -> Result<Vec<u8>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        // Alguns proxies domésticos recusam requisições sem User-Agent (HTTP 403).
        .user_agent("Aural/0.1 (Linux)")
        .build()
        .map_err(|err| err.to_string())?;
    let response = client
        .get(url)
        .send()
        .map_err(|err| err.to_string())?
        .error_for_status()
        .map_err(|err| err.to_string())?;
    let bytes = response.bytes().map_err(|err| err.to_string())?;
    Ok(bytes.to_vec())
}

fn decode_cover(bytes: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let image = image::load_from_memory(bytes).ok()?;
    let thumb = image.thumbnail(256, 256).to_rgba8();
    let (width, height) = thumb.dimensions();
    Some((width as usize, height as usize, thumb.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    #[test]
    fn download_sends_user_agent() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");
            let mut buffer = [0u8; 8192];
            let read = socket.read(&mut buffer).expect("read");
            let request = String::from_utf8_lossy(&buffer[..read]).to_ascii_lowercase();
            assert!(
                request.contains("user-agent: aural/0.1"),
                "request sem User-Agent do Aural: {request}"
            );
            let body = b"cover-bytes";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket
                .write_all(response.as_bytes())
                .expect("write headers");
            socket.write_all(body).expect("write body");
        });

        let bytes = download_bytes(&format!("http://{addr}/cover")).expect("download");
        assert_eq!(bytes, b"cover-bytes");
        server.join().expect("server");
    }
}
