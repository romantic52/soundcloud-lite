#![windows_subsystem = "windows"]

mod soundcloud;
mod ui;

use anyhow::{Context, Result};
use slint::{ComponentHandle, ModelRc, VecModel};
use soundcloud::{Account, SoundCloud, Track};
use std::{cell::RefCell, rc::Rc, sync::mpsc, thread};
use ui::{PlayerWindow, TrackRow};
use url::Url;
use windows::{
    Foundation::{TypedEventHandler, Uri},
    Media::{
        Core::MediaSource,
        Playback::{
            MediaPlaybackSession, MediaPlaybackState, MediaPlayer, MediaPlayerFailedEventArgs,
        },
    },
    Win32::System::WinRT::{RO_INIT_SINGLETHREADED, RoInitialize, RoUninitialize},
    core::{HSTRING, IInspectable},
};

enum Request {
    SignIn,
    SignOut,
    Search(String),
    Play(usize, Track),
}

enum Response {
    Status(String),
    Account(Account),
    SignedOut,
    Tracks(Vec<Track>),
    Stream(usize, Track, Url),
    Error(String, bool),
}

#[derive(Default)]
struct State {
    tracks: Vec<Track>,
    current_page: Option<Url>,
}

struct RuntimeApartment;

impl RuntimeApartment {
    fn initialize() -> windows::core::Result<Self> {
        unsafe { RoInitialize(RO_INIT_SINGLETHREADED)? };
        Ok(Self)
    }
}

impl Drop for RuntimeApartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() }
    }
}

fn main() -> Result<()> {
    let _apartment = RuntimeApartment::initialize()?;
    let player = MediaPlayer::new()?;
    player.SetVolume(0.55)?;
    let window = PlayerWindow::new()?;
    window.set_status("Войди в SoundCloud, чтобы искать музыку.".into());
    window.set_busy(true);

    attach_playback_events(&player, &window)?;
    let state = Rc::new(RefCell::new(State::default()));
    let (request_tx, request_rx) = mpsc::channel();
    let (response_tx, response_rx) = mpsc::channel();
    let notification_window = window.as_weak();
    thread::spawn(move || {
        run_network(request_rx, |response| {
            let _ = response_tx.send(response);
            let _ = notification_window.upgrade_in_event_loop(|window| {
                window.invoke_network_ready();
            });
        })
    });

    let weak = window.as_weak();
    let player_for_responses = player.clone();
    let state_for_responses = state.clone();
    window.on_network_ready(move || {
        if let Some(window) = weak.upgrade() {
            while let Ok(response) = response_rx.try_recv() {
                apply_response(
                    &window,
                    &player_for_responses,
                    &state_for_responses,
                    response,
                );
            }
        }
    });

    let weak = window.as_weak();
    let tx = request_tx.clone();
    window.on_sign_in(move || {
        dispatch(&weak, &tx, Request::SignIn, "Подключение к SoundCloud…");
    });
    let weak = window.as_weak();
    let tx = request_tx.clone();
    window.on_sign_out(move || {
        dispatch(&weak, &tx, Request::SignOut, "Выход из SoundCloud…");
    });
    let weak = window.as_weak();
    let tx = request_tx.clone();
    window.on_search(move |query| {
        let query = query.trim();
        if !query.is_empty() {
            dispatch(&weak, &tx, Request::Search(query.to_owned()), "Поиск…");
        }
    });
    let weak = window.as_weak();
    let track_state = state.clone();
    window.on_play_track(move |index| {
        if let Ok(index) = usize::try_from(index)
            && let Some(track) = track_state.borrow().tracks.get(index).cloned()
        {
            dispatch(
                &weak,
                &request_tx,
                Request::Play(index, track),
                "Открываю поток…",
            );
        }
    });

    let weak = window.as_weak();
    let player_for_toggle = player.clone();
    window.on_toggle_playback(move || {
        let result = player_for_toggle.PlaybackSession().and_then(|session| {
            if session.PlaybackState()? == MediaPlaybackState::Playing {
                player_for_toggle.Pause()
            } else {
                player_for_toggle.Play()
            }
        });
        if result.is_err()
            && let Some(window) = weak.upgrade()
        {
            window.set_status("Не удалось переключить воспроизведение.".into());
        }
    });
    let player_for_volume = player.clone();
    window.on_volume_changed(move |volume| {
        let _ = player_for_volume.SetVolume(f64::from(volume.clamp(0.0, 1.0)));
    });
    let weak = window.as_weak();
    window.on_open_track(move || {
        if let Some(url) = &state.borrow().current_page
            && open::that(url.as_str()).is_err()
            && let Some(window) = weak.upgrade()
        {
            window.set_status("Не удалось открыть страницу трека.".into());
        }
    });

    let result = window.run();
    let _ = player.Close();
    result?;
    Ok(())
}

fn dispatch(
    weak: &slint::Weak<PlayerWindow>,
    tx: &mpsc::Sender<Request>,
    request: Request,
    status: &str,
) {
    if let Some(window) = weak.upgrade()
        && !window.get_busy()
    {
        if tx.send(request).is_ok() {
            window.set_busy(true);
            window.set_status(status.into());
        } else {
            window.set_status("Заново открой плеер, чтобы подключиться к SoundCloud.".into());
        }
    }
}

fn run_network(rx: mpsc::Receiver<Request>, send: impl Fn(Response)) {
    // Opening the app is useful even before the developer credentials have been set.
    let mut cloud = match SoundCloud::open() {
        Ok(cloud) => Some(cloud),
        Err(error) => {
            send(Response::Error(error.to_string(), false));
            None
        }
    };
    if let Some(client) = &mut cloud {
        if client.has_session() {
            match client.account() {
                Ok(account) => send(Response::Account(account)),
                Err(error) => send(Response::Error(error.to_string(), false)),
            }
        } else {
            send(Response::SignedOut);
        }
    }
    for request in rx {
        let Some(client) = cloud.as_mut() else {
            send(Response::Error(
                "Задай SOUNDCLOUD_CLIENT_ID и SOUNDCLOUD_CLIENT_SECRET, затем перезапусти плеер."
                    .into(),
                false,
            ));
            continue;
        };
        let result = match request {
            Request::SignIn => {
                send(Response::Status(
                    "Заверши вход в открывшемся браузере.".into(),
                ));
                client.sign_in().map(Response::Account)
            }
            Request::SignOut => client.sign_out().map(|()| Response::SignedOut),
            Request::Search(query) => client.search(&query).map(Response::Tracks),
            Request::Play(index, track) => client
                .stream(&track)
                .map(|url| Response::Stream(index, track, url)),
        };
        match result {
            Ok(response) => send(response),
            Err(error) => send(Response::Error(error.to_string(), client.has_session())),
        }
    }
}

fn apply_response(
    window: &PlayerWindow,
    player: &MediaPlayer,
    state: &RefCell<State>,
    response: Response,
) {
    if let Response::Status(message) = response {
        window.set_status(message.into());
        return;
    }
    window.set_busy(false);
    match response {
        Response::Status(_) => unreachable!(),
        Response::Account(account) => {
            window.set_signed_in(true);
            window.set_user_name(account.username.into());
            window.set_status("".into());
        }
        Response::SignedOut => {
            clear_view(window, player, state);
            window.set_status("Войди в SoundCloud, чтобы искать музыку.".into());
        }
        Response::Tracks(tracks) => {
            let display = tracks
                .iter()
                .map(|track| TrackRow {
                    title: track.title.clone().into(),
                    artist: track.user.username.clone().into(),
                    duration: duration_label(track.duration).into(),
                })
                .collect::<Vec<_>>();
            window.set_status(if display.is_empty() {
                "Треки не найдены.".into()
            } else {
                "".into()
            });
            window.set_tracks(ModelRc::new(VecModel::from(display)));
            window.set_selected(-1);
            state.borrow_mut().tracks = tracks;
        }
        Response::Stream(index, track, url) => match set_stream(player, &url) {
            Ok(()) => {
                state.borrow_mut().current_page = Url::parse(&track.permalink_url)
                    .ok()
                    .filter(soundcloud::is_track_page);
                window.set_selected(index as i32);
                window.set_now_title(track.title.into());
                window.set_now_artist(track.user.username.into());
                window.set_can_pause(true);
                window.set_status("".into());
            }
            Err(error) => window.set_status(error.to_string().into()),
        },
        Response::Error(message, connected) => {
            if !connected {
                clear_view(window, player, state);
            }
            window.set_status(message.into());
        }
    }
}

fn clear_view(window: &PlayerWindow, player: &MediaPlayer, state: &RefCell<State>) {
    let _ = player.Pause();
    let _ = player.SetSource(None);
    *state.borrow_mut() = State::default();
    window.set_tracks(ModelRc::default());
    window.set_selected(-1);
    window.set_signed_in(false);
    window.set_user_name("".into());
    window.set_now_title("".into());
    window.set_now_artist("".into());
    window.set_playing(false);
    window.set_can_pause(false);
}

fn set_stream(player: &MediaPlayer, url: &Url) -> Result<()> {
    let source = Uri::CreateUri(&HSTRING::from(url.as_str()))
        .and_then(|uri| MediaSource::CreateFromUri(&uri))
        .context("Windows не смог открыть аудиопоток.")?;
    player
        .SetSource(&source)
        .context("Не удалось подключить аудиопоток.")?;
    player
        .Play()
        .context("Не удалось начать воспроизведение.")?;
    Ok(())
}

fn attach_playback_events(
    player: &MediaPlayer,
    window: &PlayerWindow,
) -> windows::core::Result<()> {
    let weak = window.as_weak();
    let session = player.PlaybackSession()?;
    session.PlaybackStateChanged(
        &TypedEventHandler::<MediaPlaybackSession, IInspectable>::new(move |sender, _| {
            if let Some(sender) = sender.as_ref() {
                let playing = sender.PlaybackState()? == MediaPlaybackState::Playing;
                let _ = weak.upgrade_in_event_loop(move |window| window.set_playing(playing));
            }
            Ok(())
        }),
    )?;
    let weak = window.as_weak();
    player.MediaFailed(
        &TypedEventHandler::<MediaPlayer, MediaPlayerFailedEventArgs>::new(move |_, _| {
            let _ = weak.upgrade_in_event_loop(|window| {
                if window.get_signed_in() {
                    window.set_playing(false);
                    window.set_can_pause(false);
                    window.set_status(
                        "Поток недоступен. Выбери трек ещё раз или попробуй другой.".into(),
                    );
                }
            });
            Ok(())
        }),
    )?;
    Ok(())
}

fn duration_label(milliseconds: u64) -> String {
    let seconds = milliseconds / 1000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{Duration, Instant},
    };

    #[test]
    fn windows_player_opens_and_plays_a_local_audio_file() -> Result<()> {
        let _apartment = RuntimeApartment::initialize()?;
        let mut wave = Vec::new();
        let sample_data = 16_000_u32 * 6;
        wave.extend_from_slice(b"RIFF");
        wave.extend_from_slice(&(36 + sample_data).to_le_bytes());
        wave.extend_from_slice(b"WAVEfmt ");
        wave.extend_from_slice(&16_u32.to_le_bytes());
        wave.extend_from_slice(&1_u16.to_le_bytes());
        wave.extend_from_slice(&1_u16.to_le_bytes());
        wave.extend_from_slice(&8000_u32.to_le_bytes());
        wave.extend_from_slice(&16000_u32.to_le_bytes());
        wave.extend_from_slice(&2_u16.to_le_bytes());
        wave.extend_from_slice(&16_u16.to_le_bytes());
        wave.extend_from_slice(b"data");
        wave.extend_from_slice(&sample_data.to_le_bytes());
        wave.resize(wave.len() + sample_data as usize, 0);

        let path =
            std::env::temp_dir().join(format!("soundcloud-lite-test-{}.wav", std::process::id()));
        fs::write(&path, &wave)?;
        let outcome = (|| {
            let player = MediaPlayer::new()?;
            player.SetVolume(0.0)?;
            let url =
                Url::from_file_path(&path).map_err(|_| anyhow::anyhow!("Invalid audio path"))?;
            set_stream(&player, &url)?;
            let session = player.PlaybackSession()?;
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if session.PlaybackState()? == MediaPlaybackState::Playing {
                    player.Close()?;
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(35));
            }
            let _ = player.Close();
            anyhow::bail!("Windows MediaPlayer did not reach Playing");
        })();
        let _ = fs::remove_file(path);
        outcome
    }
}
