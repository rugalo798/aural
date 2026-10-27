use anyhow::{Context, Result};
use lofty::prelude::*;
use lofty::tag::{ItemKey, Tag};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

use crate::db::{Db, StoredTrack, normalize_album, normalize_artist};
use crate::model::album_key;

const AUDIO_EXTENSIONS: &[&str] = &[
    "flac", "mp3", "m4a", "m4b", "mp4", "ogg", "oga", "wav", "aac",
];

const COVER_NAMES: &[&str] = &[
    "cover.jpg",
    "cover.jpeg",
    "cover.png",
    "folder.jpg",
    "folder.jpeg",
    "folder.png",
    "front.jpg",
    "front.jpeg",
    "front.png",
    "album.jpg",
    "album.jpeg",
    "album.png",
];

pub fn scan_library(
    folders: &[PathBuf],
    db_path: &Path,
    cover_dir: &Path,
    progress: impl Fn(usize),
) -> Result<usize> {
    fs::create_dir_all(cover_dir)
        .with_context(|| format!("criando diretório de capas {}", cover_dir.display()))?;

    let mut tracks: Vec<StoredTrack> = Vec::new();
    let mut errors = 0usize;
    let mut album_covers: HashMap<String, Option<String>> = HashMap::new();
    let mut dir_covers: HashMap<PathBuf, Option<PathBuf>> = HashMap::new();

    for folder in folders {
        if !folder.is_dir() {
            continue;
        }
        for entry in WalkDir::new(folder)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            if !AUDIO_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()) {
                continue;
            }
            match read_track(path) {
                Ok(track) => {
                    let key = album_key(&track.album, &track.album_artist);
                    let cover = match album_covers.get(&key) {
                        Some(cover) => cover.clone(),
                        None => {
                            let parent = path.parent().unwrap_or(Path::new("."));
                            let found =
                                extract_cover(path, parent, &key, cover_dir, &mut dir_covers);
                            album_covers.insert(key, found.clone());
                            found
                        }
                    };
                    let mut track = track;
                    track.cover_path = cover;
                    tracks.push(track);
                }
                Err(err) => {
                    errors += 1;
                    if errors <= 5 {
                        eprintln!("falha ao ler {}: {err:#}", path.display());
                    }
                }
            }
            if tracks.len().is_multiple_of(100) {
                progress(tracks.len());
            }
        }
    }

    progress(tracks.len());
    let mut db = Db::open(db_path)?;
    db.replace_local(&tracks)?;

    if errors > 0 {
        eprintln!("{errors} arquivos não puderam ser lidos");
    }
    Ok(tracks.len())
}

fn read_track(path: &Path) -> Result<StoredTrack> {
    let tagged = lofty::read_from_path(path)
        .with_context(|| format!("lendo metadados de {}", path.display()))?;
    let duration_ms = tagged.properties().duration().as_millis() as u64;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());

    let title = tag
        .and_then(|t| t.title())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "Faixa sem título".to_string())
        });

    let artist = tag.and_then(|t| t.artist()).map(|v| v.into_owned());
    let album = tag.and_then(|t| t.album()).map(|v| v.into_owned());
    let album_artist = tag
        .and_then(|t| t.get_string(ItemKey::AlbumArtist))
        .map(|v| v.to_string());

    let artist = normalize_artist(artist);
    let album_artist = normalize_artist(album_artist.or_else(|| Some(artist.clone())));
    let album = normalize_album(album);

    let mtime = fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    Ok(StoredTrack {
        path: path.to_string_lossy().to_string(),
        title,
        artist,
        album,
        album_artist,
        track_no: tag.and_then(|t| t.track()),
        disc_no: tag.and_then(|t| t.disk()),
        year: tag.and_then(|t| {
            t.get_string(ItemKey::RecordingDate)
                .or_else(|| t.get_string(ItemKey::Year))
                .and_then(|value| value.trim().get(0..4))
                .and_then(|value| value.parse().ok())
        }),
        genre: tag.and_then(|t| t.genre()).map(|v| v.into_owned()),
        duration_ms,
        cover_path: None,
        mtime,
    })
}

fn extract_cover(
    file: &Path,
    dir: &Path,
    album_key: &str,
    cover_dir: &Path,
    dir_covers: &mut HashMap<PathBuf, Option<PathBuf>>,
) -> Option<String> {
    if let Ok(tagged) = lofty::read_from_path(file) {
        let tag: Option<&Tag> = tagged.primary_tag().or_else(|| tagged.first_tag());
        if let Some(picture) = tag.and_then(|t| t.pictures().first()) {
            let data = picture.data();
            if !data.is_empty() {
                let hash = md5::compute(album_key.as_bytes());
                let out = cover_dir.join(format!("{hash:x}.img"));
                if fs::write(&out, data).is_ok() {
                    return Some(out.to_string_lossy().to_string());
                }
            }
        }
    }

    if let Some(found) = dir_covers.get(dir) {
        return found.as_ref().map(|p| p.to_string_lossy().to_string());
    }

    let mut found = None;
    for name in COVER_NAMES {
        let candidate = dir.join(name);
        if candidate.is_file() {
            found = Some(candidate);
            break;
        }
    }
    dir_covers.insert(dir.to_path_buf(), found.clone());
    found.map(|p| p.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_fixture_library_when_available() {
        let Some(fixture) = std::env::var_os("AURAL_TEST_MUSIC") else {
            return;
        };
        let root = std::env::temp_dir().join("aural-scan-test");
        let _ = fs::remove_dir_all(&root);
        let db_path = root.join("library.db");
        let cover_dir = root.join("covers");
        let count =
            scan_library(&[PathBuf::from(fixture)], &db_path, &cover_dir, |_| {}).expect("scan");
        assert!(count >= 3, "expected at least 3 tracks, got {count}");

        let db = Db::open(&db_path).expect("open db");
        let albums = db.albums().expect("albums");
        assert_eq!(albums.len(), 2, "expected 2 albums, got {albums:?}");
        assert!(albums.iter().all(|album| album.track_count > 0));

        let tracks = db.album_tracks(&albums[0].key).expect("album tracks");
        assert!(!tracks.is_empty());
        assert!(matches!(
            tracks[0].source,
            crate::model::TrackSource::Local(_)
        ));
    }
}
