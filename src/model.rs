use std::path::PathBuf;

pub const UNKNOWN_ALBUM: &str = "Álbum desconhecido";
pub const UNKNOWN_ARTIST: &str = "Artista desconhecido";
pub const ALBUM_KEY_SEP: char = '\u{1f}';

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrackSource {
    Local(PathBuf),
    Remote(String),
}

#[derive(Clone, Debug)]
pub struct Track {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub cover: Option<String>,
    pub source: TrackSource,
}

impl Track {
    pub fn subtitle(&self) -> String {
        if self.artist.is_empty() {
            self.album.clone()
        } else {
            self.artist.clone()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AlbumOrigin {
    Local,
    Server { server_id: i64, album_id: String },
}

#[derive(Clone, Debug)]
pub struct Album {
    pub key: String,
    pub title: String,
    pub artist: String,
    pub year: Option<u32>,
    pub cover: Option<String>,
    pub track_count: u32,
    pub duration_ms: u64,
    pub origin: AlbumOrigin,
}

impl Album {
    pub fn is_server(&self) -> bool {
        matches!(self.origin, AlbumOrigin::Server { .. })
    }
}

#[derive(Clone, Debug)]
pub struct AlbumDetail {
    pub album: Album,
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug)]
pub struct ArtistRow {
    pub name: String,
    pub album_count: u32,
    pub track_count: u32,
}

pub fn album_key(album: &str, album_artist: &str) -> String {
    format!("{album}{ALBUM_KEY_SEP}{album_artist}")
}

pub fn server_album_key(server_id: i64, album_id: &str) -> String {
    format!("srv:{server_id}:{album_id}")
}

pub fn fmt_duration(ms: u64) -> String {
    let total_secs = ms / 1000;
    let secs = total_secs % 60;
    let mins = total_secs / 60;
    format!("{mins}:{secs:02}")
}
