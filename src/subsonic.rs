use anyhow::{Context, Result, anyhow, bail};
use reqwest::Url;
use reqwest::blocking::Client;
use serde_json::Value;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config::ServerConfig;

#[derive(Clone, Debug)]
pub struct SubAlbum {
    pub id: String,
    pub name: String,
    pub artist: String,
    pub year: Option<u32>,
    pub cover_id: Option<String>,
    pub song_count: u32,
    pub duration_ms: u64,
}

#[derive(Clone, Debug)]
pub struct SubSong {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub track: Option<u32>,
    pub disc: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct SubAlbumDetail {
    pub album: SubAlbum,
    pub songs: Vec<SubSong>,
}

pub struct SubsonicClient {
    base: String,
    user: String,
    password: String,
    http: Client,
}

impl SubsonicClient {
    pub fn new(cfg: &ServerConfig) -> Result<Self> {
        let mut base = cfg.url.trim().trim_end_matches('/').to_string();
        if base.is_empty() {
            bail!("URL do servidor vazia");
        }
        if !base.starts_with("http://") && !base.starts_with("https://") {
            base = format!("http://{base}");
        }
        let http = Client::builder()
            .timeout(Duration::from_secs(25))
            .user_agent("Aural/0.1 (Linux)")
            .build()
            .context("criando cliente HTTP")?;
        Ok(Self {
            base,
            user: cfg.username.clone(),
            password: cfg.password.clone(),
            http,
        })
    }

    fn auth_pairs(&self) -> Vec<(String, String)> {
        let salt = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let salt = format!("{salt:x}");
        let token = md5::compute(format!("{}{}", self.password, salt));
        vec![
            ("u".to_string(), self.user.clone()),
            ("t".to_string(), format!("{token:x}")),
            ("s".to_string(), salt),
            ("v".to_string(), "1.16.1".to_string()),
            ("c".to_string(), "Aural".to_string()),
            ("f".to_string(), "json".to_string()),
        ]
    }

    fn url(&self, endpoint: &str, params: &[(&str, String)]) -> Result<Url> {
        let raw = format!("{}/rest/{}", self.base, endpoint);
        let mut url = Url::parse(&raw).with_context(|| format!("URL inválida: {raw}"))?;
        {
            let mut query = url.query_pairs_mut();
            for (k, v) in self.auth_pairs() {
                query.append_pair(&k, &v);
            }
            for (k, v) in params {
                query.append_pair(k, v);
            }
        }
        Ok(url)
    }

    fn get(&self, endpoint: &str, params: &[(&str, String)]) -> Result<Value> {
        let url = self.url(endpoint, params)?;
        let response = self
            .http
            .get(url)
            .send()
            .with_context(|| format!("requisição {endpoint} falhou"))?
            .error_for_status()
            .with_context(|| format!("servidor recusou {endpoint}"))?;
        let value: Value = response
            .json()
            .with_context(|| format!("resposta inválida de {endpoint}"))?;
        let root = value
            .get("subsonic-response")
            .ok_or_else(|| anyhow!("resposta sem subsonic-response"))?;
        if root.get("status").and_then(Value::as_str) != Some("ok") {
            let message = root
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("erro retornado pelo servidor");
            bail!("{message}");
        }
        Ok(root.clone())
    }

    pub fn ping(&self) -> Result<()> {
        self.get("ping.view", &[])?;
        Ok(())
    }

    pub fn albums(&self) -> Result<Vec<SubAlbum>> {
        let root = self.get(
            "getAlbumList2.view",
            &[
                ("type", "alphabeticalByName".to_string()),
                ("size", "500".to_string()),
                ("offset", "0".to_string()),
            ],
        )?;
        let list = root
            .pointer("/albumList2/album")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(list.iter().map(parse_album).collect())
    }

    pub fn album(&self, id: &str) -> Result<SubAlbumDetail> {
        let root = self.get("getAlbum.view", &[("id", id.to_string())])?;
        let album_value = root
            .get("album")
            .ok_or_else(|| anyhow!("álbum não encontrado"))?;
        let album = parse_album(album_value);
        let songs = album_value
            .get("song")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(parse_song)
            .collect();
        Ok(SubAlbumDetail { album, songs })
    }

    pub fn stream_url(&self, id: &str) -> Result<String> {
        let url = self.url(
            "stream.view",
            &[("id", id.to_string()), ("format", "raw".to_string())],
        )?;
        Ok(url.to_string())
    }

    pub fn cover_url(&self, id: &str, size: u32) -> Result<String> {
        let url = self.url(
            "getCoverArt.view",
            &[("id", id.to_string()), ("size", size.to_string())],
        )?;
        Ok(url.to_string())
    }
}

fn parse_album(value: &Value) -> SubAlbum {
    SubAlbum {
        id: string_of(value.get("id")),
        name: string_of(value.get("name")).if_empty("Álbum sem nome"),
        artist: string_of(value.get("artist")).if_empty("Artista desconhecido"),
        year: number_of(value.get("year")).map(|n| n as u32),
        cover_id: value
            .get("coverArt")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        song_count: number_of(value.get("songCount")).unwrap_or(0) as u32,
        duration_ms: number_of(value.get("duration")).unwrap_or(0) * 1000,
    }
}

fn parse_song(value: &Value) -> SubSong {
    SubSong {
        id: string_of(value.get("id")),
        title: string_of(value.get("title")).if_empty("Faixa sem título"),
        artist: string_of(value.get("artist")),
        album: string_of(value.get("album")),
        duration_ms: number_of(value.get("duration")).unwrap_or(0) * 1000,
        track: number_of(value.get("track")).map(|n| n as u32),
        disc: number_of(value.get("discNumber")).map(|n| n as u32),
    }
}

fn string_of(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn number_of(value: Option<&Value>) -> Option<u64> {
    match value {
        Some(Value::Number(n)) => n.as_u64(),
        Some(Value::String(s)) => s.parse().ok(),
        _ => None,
    }
}

trait IfEmpty {
    fn if_empty(self, fallback: &str) -> String;
}

impl IfEmpty for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.trim().is_empty() {
            fallback.to_string()
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn parses_subsonic_responses_and_signs_requests() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = thread::spawn(move || {
            for _ in 0..3 {
                let (mut socket, _) = listener.accept().expect("accept");
                let mut buffer = [0u8; 8192];
                let read = socket.read(&mut buffer).expect("read");
                let request = String::from_utf8_lossy(&buffer[..read]).to_string();
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .expect("request path")
                    .to_string();

                let url =
                    reqwest::Url::parse(&format!("http://localhost{path}")).expect("request url");
                let params: HashMap<String, String> = url
                    .query_pairs()
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                assert_eq!(params.get("u").map(String::as_str), Some("user"));
                assert_eq!(params.get("c").map(String::as_str), Some("Aural"));
                let salt = params.get("s").expect("salt");
                let expected = format!("{:x}", md5::compute(format!("pass{salt}")));
                assert_eq!(params.get("t"), Some(&expected));

                let body = if path.contains("ping") {
                    r#"{"subsonic-response":{"status":"ok","version":"1.16.1"}}"#.to_string()
                } else if path.contains("getAlbumList2") {
                    r#"{"subsonic-response":{"status":"ok","albumList2":{"album":[{"id":"al1","name":"Server Album","artist":"Server Artist","year":2024,"coverArt":"al1","songCount":2,"duration":9}]}}}"#.to_string()
                } else if path.contains("getAlbum") {
                    r#"{"subsonic-response":{"status":"ok","album":{"id":"al1","name":"Server Album","artist":"Server Artist","year":2024,"coverArt":"al1","songCount":2,"duration":9,"song":[{"id":"s1","title":"Track One","artist":"Server Artist","album":"Server Album","duration":4,"track":1,"discNumber":1},{"id":"s2","title":"Track Two","artist":"Server Artist","album":"Server Album","duration":5,"track":2,"discNumber":1}]}}}"#.to_string()
                } else {
                    r#"{"subsonic-response":{"status":"failed","error":{"message":"not found"}}}"#
                        .to_string()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                socket.write_all(response.as_bytes()).expect("write");
            }
        });

        let config = ServerConfig {
            id: 7,
            name: "Mock".to_string(),
            url: format!("http://{addr}"),
            username: "user".to_string(),
            password: "pass".to_string(),
        };
        let client = SubsonicClient::new(&config).expect("client");
        client.ping().expect("ping");

        let albums = client.albums().expect("albums");
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].id, "al1");
        assert_eq!(albums[0].name, "Server Album");
        assert_eq!(albums[0].song_count, 2);

        let detail = client.album("al1").expect("album");
        assert_eq!(detail.songs.len(), 2);
        assert_eq!(detail.songs[0].title, "Track One");
        assert_eq!(detail.songs[1].disc, Some(1));

        let stream = client.stream_url("s1").expect("stream url");
        assert!(stream.contains("/rest/stream.view"));
        assert!(stream.contains("id=s1"));
        assert!(stream.contains("t="));

        server.join().expect("server");
    }
}
