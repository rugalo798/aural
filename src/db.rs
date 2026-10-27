use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};

use crate::model::{
    Album, AlbumOrigin, ArtistRow, Track, TrackSource, UNKNOWN_ALBUM, UNKNOWN_ARTIST, album_key,
};

#[derive(Clone, Debug)]
pub struct StoredTrack {
    pub path: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track_no: Option<u32>,
    pub disc_no: Option<u32>,
    pub year: Option<u32>,
    pub genre: Option<String>,
    pub duration_ms: u64,
    pub cover_path: Option<String>,
    pub mtime: i64,
}

pub struct Db {
    conn: Connection,
    path: PathBuf,
}

const ALBUM_EXPR: &str = "COALESCE(NULLIF(album,''), 'Álbum desconhecido')";
const ARTIST_EXPR: &str =
    "COALESCE(NULLIF(album_artist,''), NULLIF(artist,''), 'Artista desconhecido')";

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("criando diretório {}", parent.display()))?;
        }
        let conn =
            Connection::open(path).with_context(|| format!("abrindo banco {}", path.display()))?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;\nPRAGMA synchronous=NORMAL;\nPRAGMA busy_timeout=5000;\nPRAGMA foreign_keys=ON;",
        )?;
        let db = Self {
            conn,
            path: path.to_path_buf(),
        };
        db.init()?;
        Ok(db)
    }

    fn init(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS tracks (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                path         TEXT NOT NULL UNIQUE,
                title        TEXT NOT NULL,
                artist       TEXT NOT NULL,
                album        TEXT NOT NULL,
                album_artist TEXT NOT NULL,
                track_no     INTEGER,
                disc_no      INTEGER,
                year         INTEGER,
                genre        TEXT,
                duration_ms  INTEGER NOT NULL DEFAULT 0,
                cover_path   TEXT,
                mtime        INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_tracks_album  ON tracks(album, album_artist);
            CREATE INDEX IF NOT EXISTS idx_tracks_artist ON tracks(album_artist);
            "#,
        )?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn replace_local(&mut self, tracks: &[StoredTrack]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM tracks", [])?;
        {
            let mut stmt = tx.prepare(
                r#"INSERT OR REPLACE INTO tracks
                   (path, title, artist, album, album_artist, track_no, disc_no, year, genre, duration_ms, cover_path, mtime)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"#,
            )?;
            for t in tracks {
                stmt.execute(params![
                    t.path,
                    t.title,
                    t.artist,
                    t.album,
                    t.album_artist,
                    t.track_no,
                    t.disc_no,
                    t.year,
                    t.genre,
                    t.duration_ms as i64,
                    t.cover_path,
                    t.mtime,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn track_count(&self) -> Result<u64> {
        let n: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))
            .unwrap_or(0);
        Ok(n.max(0) as u64)
    }

    pub fn albums(&self) -> Result<Vec<Album>> {
        let sql = format!(
            r#"SELECT {ALBUM_EXPR} AS alb,
                      {ARTIST_EXPR} AS aa,
                      MIN(year) AS yr,
                      MAX(cover_path) AS cover,
                      COUNT(*) AS cnt,
                      COALESCE(SUM(duration_ms), 0) AS dur
               FROM tracks
               GROUP BY alb, aa
               ORDER BY aa COLLATE NOCASE, alb COLLATE NOCASE"#
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let title: String = row.get(0)?;
            let artist: String = row.get(1)?;
            let year: Option<i64> = row.get(2)?;
            let cover: Option<String> = row.get(3)?;
            let count: i64 = row.get(4)?;
            let duration: i64 = row.get(5)?;
            Ok(Album {
                key: album_key(&title, &artist),
                title,
                artist,
                year: year.and_then(|y| u32::try_from(y).ok()),
                cover,
                track_count: count.max(0) as u32,
                duration_ms: duration.max(0) as u64,
                origin: AlbumOrigin::Local,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn album_tracks(&self, key: &str) -> Result<Vec<Track>> {
        let sql = format!(
            r#"SELECT title, artist, album, duration_ms, track_no, disc_no, path, cover_path
               FROM tracks
               WHERE ({ALBUM_EXPR} || char(31) || {ARTIST_EXPR}) = ?1
               ORDER BY COALESCE(disc_no, 1), COALESCE(track_no, 9999), title COLLATE NOCASE"#
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([key], |row| {
            let title: String = row.get(0)?;
            let artist: String = row.get(1)?;
            let album: String = row.get(2)?;
            let duration: i64 = row.get(3)?;
            let track_no: Option<i64> = row.get(4)?;
            let disc_no: Option<i64> = row.get(5)?;
            let path: String = row.get(6)?;
            let cover: Option<String> = row.get(7)?;
            Ok(Track {
                title,
                artist,
                album,
                duration_ms: duration.max(0) as u64,
                track_no: track_no.and_then(|n| u32::try_from(n).ok()),
                disc_no: disc_no.and_then(|n| u32::try_from(n).ok()),
                cover,
                source: TrackSource::Local(PathBuf::from(path)),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn artists(&self) -> Result<Vec<ArtistRow>> {
        let sql = format!(
            r#"SELECT {ARTIST_EXPR} AS aa,
                      COUNT(DISTINCT {ALBUM_EXPR}) AS albums,
                      COUNT(*) AS tracks
               FROM tracks
               GROUP BY aa
               ORDER BY aa COLLATE NOCASE"#
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |row| {
            let name: String = row.get(0)?;
            let albums: i64 = row.get(1)?;
            let tracks: i64 = row.get(2)?;
            Ok(ArtistRow {
                name,
                album_count: albums.max(0) as u32,
                track_count: tracks.max(0) as u32,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn artist_tracks(&self, name: &str) -> Result<Vec<Track>> {
        let sql = format!(
            r#"SELECT title, artist, album, duration_ms, track_no, disc_no, path, cover_path
               FROM tracks
               WHERE {ARTIST_EXPR} = ?1
               ORDER BY album COLLATE NOCASE, COALESCE(disc_no, 1), COALESCE(track_no, 9999), title COLLATE NOCASE"#
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([name], |row| {
            let title: String = row.get(0)?;
            let artist: String = row.get(1)?;
            let album: String = row.get(2)?;
            let duration: i64 = row.get(3)?;
            let track_no: Option<i64> = row.get(4)?;
            let disc_no: Option<i64> = row.get(5)?;
            let path: String = row.get(6)?;
            let cover: Option<String> = row.get(7)?;
            Ok(Track {
                title,
                artist,
                album,
                duration_ms: duration.max(0) as u64,
                track_no: track_no.and_then(|n| u32::try_from(n).ok()),
                disc_no: disc_no.and_then(|n| u32::try_from(n).ok()),
                cover,
                source: TrackSource::Local(PathBuf::from(path)),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn search_albums(&self, query: &str) -> Result<Vec<Album>> {
        let like = format!("%{}%", query.replace('%', "\\%").replace('_', "\\_"));
        let sql = format!(
            r#"SELECT alb, aa, MIN(year) AS yr, MAX(cover_path) AS cover, COUNT(*) AS cnt,
                      COALESCE(SUM(duration_ms), 0) AS dur
               FROM (
                    SELECT {ALBUM_EXPR} AS alb, {ARTIST_EXPR} AS aa, year, cover_path, duration_ms
                    FROM tracks
               )
               WHERE alb LIKE ?1 ESCAPE '\' COLLATE NOCASE
                  OR aa  LIKE ?1 ESCAPE '\' COLLATE NOCASE
               GROUP BY alb, aa
               ORDER BY aa COLLATE NOCASE, alb COLLATE NOCASE
               LIMIT 200"#
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([like], |row| {
            let title: String = row.get(0)?;
            let artist: String = row.get(1)?;
            let year: Option<i64> = row.get(2)?;
            let cover: Option<String> = row.get(3)?;
            let count: i64 = row.get(4)?;
            let duration: i64 = row.get(5)?;
            Ok(Album {
                key: album_key(&title, &artist),
                title,
                artist,
                year: year.and_then(|y| u32::try_from(y).ok()),
                cover,
                track_count: count.max(0) as u32,
                duration_ms: duration.max(0) as u64,
                origin: AlbumOrigin::Local,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

pub fn normalize_album(value: Option<String>) -> String {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| UNKNOWN_ALBUM.to_string())
}

pub fn normalize_artist(value: Option<String>) -> String {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| UNKNOWN_ARTIST.to_string())
}
