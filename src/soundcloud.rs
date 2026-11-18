use anyhow::{Context, Result, anyhow, bail, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    env,
    io::{BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use url::Url;

const API: &str = "https://api.soundcloud.com";
const AUTHORIZE: &str = "https://secure.soundcloud.com/authorize";
const TOKEN: &str = "https://secure.soundcloud.com/oauth/token";
const DEFAULT_CALLBACK: &str = "http://127.0.0.1:8767/callback";
const AUDIO_ERROR: &str = "SoundCloud не предоставил поток для этого трека.";

pub struct SoundCloud {
    http: ureq::Agent,
    id: String,
    secret: String,
    callback: Url,
    vault: keyring::Entry,
    session: Option<Session>,
}

#[derive(Deserialize, Serialize)]
struct Session {
    access_token: String,
    refresh_token: String,
    expires_at: u64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
}

#[derive(Clone, Deserialize)]
pub struct Track {
    pub urn: String,
    pub title: String,
    pub duration: u64,
    pub permalink_url: String,
    pub user: Account,
}

#[derive(Clone, Deserialize)]
pub struct Account {
    pub username: String,
}

#[derive(Deserialize)]
struct TrackPage {
    collection: Vec<Track>,
}

#[derive(Deserialize)]
struct Streams {
    hls_aac_160_url: Option<String>,
    hls_aac_96_url: Option<String>,
}

impl SoundCloud {
    pub fn open() -> Result<Self> {
        let id = env::var("SOUNDCLOUD_CLIENT_ID").unwrap_or_default();
        let secret = env::var("SOUNDCLOUD_CLIENT_SECRET").unwrap_or_default();
        ensure!(
            !id.trim().is_empty() && !secret.trim().is_empty(),
            "Для входа нужны Client ID и Client Secret приложения SoundCloud."
        );
        let callback = Url::parse(
            &env::var("SOUNDCLOUD_REDIRECT_URI").unwrap_or_else(|_| DEFAULT_CALLBACK.into()),
        )
        .map_err(|_| anyhow!("Некорректный адрес возврата OAuth."))?;
        ensure!(
            callback.scheme() == "http"
                && callback.host_str() == Some("127.0.0.1")
                && callback.port().is_some()
                && !callback.path().eq("/")
                && callback.username().is_empty()
                && callback.password().is_none()
                && callback.query().is_none()
                && callback.fragment().is_none(),
            "Адрес возврата должен быть зарегистрированным HTTP-адресом на 127.0.0.1 с портом и путём."
        );
        let vault = keyring::Entry::new("SoundCloud Lite", &id)
            .context("Не удалось открыть хранилище учётных данных Windows.")?;
        let session = match vault.get_password() {
            Ok(raw) => serde_json::from_str(&raw).ok(),
            Err(keyring::Error::NoEntry) => None,
            Err(_) => bail!("Не удалось прочитать сессию из хранилища Windows."),
        };
        Ok(Self {
            http: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(12))
                .redirects(0)
                .build(),
            id,
            secret,
            callback,
            vault,
            session,
        })
    }

    pub fn has_session(&self) -> bool {
        self.session.is_some()
    }

    pub fn sign_in(&mut self) -> Result<Account> {
        let verifier = nonce();
        let state = nonce();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, self.callback.port().unwrap()))
            .context("Порт для входа занят. Закрой другой экземпляр плеера и повтори.")?;
        listener.set_nonblocking(true)?;

        let mut login_url = Url::parse(AUTHORIZE)?;
        login_url.query_pairs_mut().extend_pairs([
            ("client_id", self.id.as_str()),
            ("redirect_uri", self.callback.as_str()),
            ("response_type", "code"),
            ("code_challenge_method", "S256"),
            ("code_challenge", challenge.as_str()),
            ("state", state.as_str()),
        ]);
        open::that(login_url.as_str()).context("Не удалось открыть браузер.")?;
        let code = wait_for_code(listener, &self.callback, &state)?;
        self.exchange(&[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &self.callback.to_string()),
            ("code_verifier", &verifier),
        ])?;
        self.account()
    }

    pub fn account(&mut self) -> Result<Account> {
        self.get_json(&format!("{API}/me"))
    }

    pub fn search(&mut self, query: &str) -> Result<Vec<Track>> {
        let mut url = Url::parse(&format!("{API}/tracks"))?;
        url.query_pairs_mut().extend_pairs([
            ("q", query),
            ("access", "playable"),
            ("limit", "30"),
            ("linked_partitioning", "true"),
        ]);
        Ok(self.get_json::<TrackPage>(url.as_str())?.collection)
    }

    pub fn stream(&mut self, track: &Track) -> Result<Url> {
        let mut url = Url::parse(API)?;
        url.path_segments_mut()
            .map_err(|_| anyhow!(AUDIO_ERROR))?
            .extend(["tracks", &track.urn, "streams"]);
        let streams: Streams = self.get_json(url.as_str())?;
        for stream in [streams.hls_aac_160_url, streams.hls_aac_96_url]
            .into_iter()
            .flatten()
        {
            if let Ok(url) = self.resolve_stream(&stream) {
                return Ok(url);
            }
        }
        bail!(AUDIO_ERROR)
    }

    pub fn sign_out(&mut self) -> Result<()> {
        if self.has_session() {
            let revoke = self.token().and_then(|token| {
                self.http
                    .post(&format!("{API}/disconnect"))
                    .set("Authorization", &format!("OAuth {token}"))
                    .call()
                    .map_err(|error| request_error(&error))
            });
            self.clear_session()?;
            if revoke.is_err() {
                bail!("Сессия удалена с ПК. SoundCloud не подтвердил отзыв доступа.")
            }
        }
        Ok(())
    }

    fn clear_session(&mut self) -> Result<()> {
        self.session = None;
        match self.vault.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => bail!("Не удалось удалить сохранённую сессию Windows."),
        }
    }

    fn exchange(&mut self, extra: &[(&str, &str)]) -> Result<()> {
        let mut form = vec![
            ("client_id", self.id.as_str()),
            ("client_secret", self.secret.as_str()),
        ];
        form.extend_from_slice(extra);
        let response = self
            .http
            .post(TOKEN)
            .set("Accept", "application/json")
            .send_form(&form);
        let payload: TokenResponse = match response {
            Ok(response) => response
                .into_json()
                .context("Некорректный ответ авторизации.")?,
            Err(ureq::Error::Status(400 | 401, _)) => {
                if extra.contains(&("grant_type", "refresh_token")) {
                    self.clear_session()?;
                }
                bail!(
                    "SoundCloud отклонил авторизацию. Проверь регистрацию приложения и повтори вход."
                )
            }
            Err(error) => return Err(request_error(&error)),
        };
        let session = Session {
            access_token: payload.access_token,
            refresh_token: payload.refresh_token,
            expires_at: now().saturating_add(payload.expires_in),
        };
        if self
            .vault
            .set_password(&serde_json::to_string(&session)?)
            .is_err()
        {
            self.session = Some(session);
            bail!("Не удалось сохранить сессию в хранилище Windows.")
        }
        self.session = Some(session);
        Ok(())
    }

    fn refresh(&mut self) -> Result<()> {
        let refresh_token = self
            .session
            .as_ref()
            .context("Сначала войди в SoundCloud.")?
            .refresh_token
            .clone();
        self.exchange(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh_token),
        ])
    }

    fn token(&mut self) -> Result<String> {
        let session = self
            .session
            .as_ref()
            .context("Сначала войди в SoundCloud.")?;
        if session.expires_at <= now().saturating_add(30) {
            self.refresh()?;
        }
        Ok(self
            .session
            .as_ref()
            .context("Сессия истекла.")?
            .access_token
            .clone())
    }

    fn api_get(&mut self, url: &str) -> Result<ureq::Response> {
        let parsed = Url::parse(url)?;
        ensure!(
            parsed.origin() == Url::parse(API)?.origin(),
            "Недопустимый адрес API."
        );
        let token = self.token()?;
        let request = |token: &str| {
            self.http
                .get(url)
                .set("Authorization", &format!("OAuth {token}"))
                .call()
        };
        match request(&token) {
            Err(ureq::Error::Status(401, _)) => {
                self.refresh()?;
                let token = self.token()?;
                self.http
                    .get(url)
                    .set("Authorization", &format!("OAuth {token}"))
                    .call()
                    .map_err(|error| request_error(&error))
            }
            result => result.map_err(|error| request_error(&error)),
        }
    }

    fn get_json<T: DeserializeOwned>(&mut self, url: &str) -> Result<T> {
        let response = self.api_get(url)?;
        ensure!(response.status() == 200, "Неожиданный ответ SoundCloud.");
        response
            .into_json()
            .context("Не удалось прочитать ответ SoundCloud.")
    }

    fn resolve_stream(&mut self, address: &str) -> Result<Url> {
        let url = Url::parse(address)?;
        if is_media_url(&url) {
            return Ok(url);
        }
        let response = self.api_get(url.as_str())?;
        ensure!((300..400).contains(&response.status()), AUDIO_ERROR);
        let target = url.join(response.header("Location").context(AUDIO_ERROR)?)?;
        ensure!(is_media_url(&target), AUDIO_ERROR);
        Ok(target)
    }
}

pub fn is_track_page(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(
            url.host_str(),
            Some("soundcloud.com" | "www.soundcloud.com")
        )
}

fn is_media_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.host_str().is_some_and(|host| {
            host.ends_with(".sndcdn.com") || host.ends_with(".media-streaming.soundcloud.cloud")
        })
}

fn request_error(error: &ureq::Error) -> anyhow::Error {
    match error {
        ureq::Error::Status(401, _) => anyhow!("Сессия истекла. Войди снова."),
        ureq::Error::Status(403, _) => anyhow!("SoundCloud отказал в доступе."),
        ureq::Error::Status(429, _) => anyhow!("Слишком много запросов. Попробуй чуть позже."),
        ureq::Error::Status(code, _) => anyhow!("SoundCloud вернул ошибку HTTP {code}."),
        ureq::Error::Transport(_) => anyhow!("Не удалось связаться с SoundCloud. Проверь сеть."),
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn nonce() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}

fn wait_for_code(listener: TcpListener, callback: &Url, state: &str) -> Result<String> {
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, _)) => {
                if let Ok(result) = accept_callback(&mut stream, callback, state) {
                    return result;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(80));
            }
            Err(error) => return Err(error).context("Не удалось принять ответ браузера."),
        }
    }
    bail!("Время входа истекло. Попробуй ещё раз.")
}

fn accept_callback(stream: &mut TcpStream, callback: &Url, state: &str) -> Result<Result<String>> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut request = String::new();
    BufReader::new(&mut *stream)
        .take(8192)
        .read_line(&mut request)?;
    let mut parts = request.split_ascii_whitespace();
    ensure!(parts.next() == Some("GET"), "Invalid OAuth callback method");
    let target = parts
        .next()
        .filter(|target| target.starts_with('/') && !target.starts_with("//"))
        .context("Invalid OAuth callback address")?;
    let received = callback.join(target)?;
    ensure!(
        received.origin() == callback.origin() && received.path() == callback.path(),
        "Invalid OAuth callback path"
    );
    let params = received
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    if !params
        .get("state")
        .is_some_and(|received| received == state)
    {
        let _ = respond(
            stream,
            "400 Bad Request",
            "Не удалось проверить ответ авторизации.",
        );
        bail!("Invalid OAuth callback state");
    }
    let code = params
        .get("code")
        .filter(|code| !code.is_empty())
        .map(ToString::to_string);
    if let Some(code) = code {
        let _ = respond(stream, "200 OK", "Ответ получен. Можно вернуться в плеер.");
        return Ok(Ok(code));
    }
    let _ = respond(stream, "400 Bad Request", "Вход в SoundCloud отменён.");
    Ok(Err(anyhow!("Вход в SoundCloud отменён.")))
}

fn respond(stream: &mut TcpStream, status: &str, message: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{message}",
        message.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_soundcloud_media_hosts_are_accepted() {
        for valid in [
            "https://playback.media-streaming.soundcloud.cloud/example.m3u8",
            "https://cf-hls-media.sndcdn.com/example.m3u8",
        ] {
            assert!(is_media_url(&Url::parse(valid).unwrap()));
        }
        for invalid in [
            "http://cf-hls-media.sndcdn.com/example.m3u8",
            "https://cf-hls-media.sndcdn.com.example.org/example.m3u8",
            "https://cf-hls-media.sndcdn.com@example.org/example.m3u8",
        ] {
            assert!(!is_media_url(&Url::parse(invalid).unwrap()));
        }
    }

    #[test]
    fn callback_rejects_wrong_state_and_accepts_the_real_code() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let callback = Url::parse(&format!("http://{address}/callback")).unwrap();

        let browser = thread::spawn(move || {
            for (target, status) in [
                ("/callback?state=wrong&code=ignored", "400 Bad Request"),
                ("/callback?state=expected&code=valid", "200 OK"),
            ] {
                let mut connection = TcpStream::connect(address).unwrap();
                connection
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                write!(connection, "GET {target} HTTP/1.1\r\n\r\n").unwrap();
                let mut response = String::new();
                connection.read_to_string(&mut response).unwrap();
                assert!(response.starts_with(&format!("HTTP/1.1 {status}")));
            }
        });
        assert_eq!(
            wait_for_code(listener, &callback, "expected").unwrap(),
            "valid"
        );
        browser.join().unwrap();
    }
}
