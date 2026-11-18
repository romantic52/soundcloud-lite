<p align="center">
  <img src="assets/logo.png" width="112" height="112" alt="Логотип SoundCloud Lite">
</p>

<h1 align="center">SoundCloud Lite</h1>

Неофициальное приложение для Windows на Rust и WebView2. Оно открывает сайт SoundCloud в отдельном окне с поиском, лентой, библиотекой, плейлистами и входом в аккаунт.

## Запуск

1. Скачай `SoundCloud-Lite.exe` из [Releases](../../releases/latest) и запусти его.
2. Войди в аккаунт SoundCloud в окне приложения, чтобы открыть свою библиотеку. API-ключ для этого режима не нужен.

Для запуска нужен [Microsoft Edge WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/). Сеанс сайта хранится в профиле `%LOCALAPPDATA%\SoundCloudLite\WebView2`.

Режим `--widget` открывает упрощённый плеер для публичных треков и плейлистов по ссылке.

```powershell
.\SoundCloud-Lite.exe --widget
```

## Сборка

Нужны Rust для Windows с MSVC и Visual Studio Build Tools. Из корня проекта выполни

```powershell
cargo build --locked --release
.\target\release\soundcloud-lite.exe
```

## Режим с собственным API-приложением

Исходный плеер на Slint и Windows MediaPlayer собирается командой `cargo build --locked --release --features native-api --bin soundcloud-lite-api`.

1. Зарегистрируй приложение в [SoundCloud Your Apps](https://soundcloud.com/you/apps). Для регистрации SoundCloud требует [Artist Pro](https://developers.soundcloud.com/docs/api/register-app).
2. Укажи адрес возврата `http://127.0.0.1:8767/callback`. Если в приложении уже зарегистрирован другой локальный адрес, укажи его в `SOUNDCLOUD_REDIRECT_URI`.
3. Из папки плеера выполни в PowerShell:

   ```powershell
   $env:SOUNDCLOUD_CLIENT_ID = Read-Host "Client ID"
   $secret = Read-Host "Client Secret" -AsSecureString
   $env:SOUNDCLOUD_CLIENT_SECRET = [System.Net.NetworkCredential]::new('', $secret).Password
   & ".\target\release\soundcloud-lite-api.exe"
   ```

4. Нажми **Войти**. Вход завершится в браузере SoundCloud. Плеер хранит токены в диспетчере учётных данных Windows.

Плеер показывает композиции, которые SoundCloud разрешает воспроизводить через API сторонним приложениям.
