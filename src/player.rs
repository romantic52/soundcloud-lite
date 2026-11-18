#![windows_subsystem = "windows"]

use std::{env, error::Error, io, path::PathBuf};
use url::Url;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    ICoreWebView2_19,
};
use windows::{
    Win32::Foundation::{HWND, WPARAM},
    Win32::UI::WindowsAndMessaging::{
        GCLP_HICON, GCLP_HICONSM, GetSystemMetrics, ICON_BIG, ICON_SMALL, MB_ICONERROR, MB_OK,
        MessageBoxW, SM_CXSMICON, SM_CYSMICON, SendMessageW, SetClassLongPtrW, WM_GETICON,
    },
    core::{Interface, PCWSTR, w},
};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalSize, PhysicalSize},
    event::WindowEvent,
    event_loop::{ActiveEventLoop, EventLoop},
    platform::windows::{IconExtWindows, WindowAttributesExtWindows},
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::{Icon, Window, WindowId},
};
use wry::{NewWindowResponse, WebContext, WebView, WebViewBuilder, WebViewExtWindows};

const HOME: &str = "https://soundcloud.com/discover";
const WIDGET: &str = include_str!("player.html");

struct Player {
    view: Option<WebView>,
    window: Option<Window>,
    context: WebContext,
    widget_mode: bool,
}

impl Player {
    fn open(&mut self, event_loop: &ActiveEventLoop) -> Result<(), Box<dyn Error>> {
        let size = if self.widget_mode {
            LogicalSize::new(760, 550)
        } else {
            LogicalSize::new(1080, 730)
        };
        // The icon compiled into the executable by build.rs has resource ID 1.
        let small_size = unsafe {
            PhysicalSize::new(
                GetSystemMetrics(SM_CXSMICON) as u32,
                GetSystemMetrics(SM_CYSMICON) as u32,
            )
        };
        let small_icon = Icon::from_resource(1, Some(small_size)).ok();
        let taskbar_icon = Icon::from_resource(1, None).ok();
        let window = event_loop.create_window(
            Window::default_attributes()
                .with_title("SoundCloud Lite")
                .with_class_name("SoundCloudLite.MainWindow")
                .with_visible(false)
                .with_inner_size(size)
                .with_min_inner_size(LogicalSize::new(450, 380))
                .with_window_icon(small_icon)
                .with_taskbar_icon(taskbar_icon),
        )?;
        set_class_icons(&window)?;
        let builder = WebViewBuilder::new_with_web_context(&mut self.context);
        let builder = if self.widget_mode {
            builder.with_html(WIDGET).with_navigation_handler(|url| {
                url == "about:blank" || url.starts_with("data:text/html")
            })
        } else {
            builder.with_url(HOME).with_navigation_handler(|url| {
                url == "about:blank" || is_soundcloud_page(&url) || is_sign_in_page(&url)
            })
        };
        let view = builder
            .with_new_window_req_handler(|url, _| {
                if url == "about:blank" || is_soundcloud_page(&url) || is_sign_in_page(&url) {
                    NewWindowResponse::Allow
                } else {
                    NewWindowResponse::Deny
                }
            })
            .build(&window)?;

        window.set_visible(true);
        self.window = Some(window);
        self.view = Some(view);
        Ok(())
    }
}

fn set_class_icons(window: &Window) -> Result<(), Box<dyn Error>> {
    let RawWindowHandle::Win32(handle) = window.window_handle()?.as_raw() else {
        return Err(io::Error::other("SoundCloud Lite requires a Win32 window").into());
    };
    let hwnd = HWND(handle.hwnd.get() as _);
    // The window owns these handles for its lifetime. The shell also queries the class icons.
    unsafe {
        for (size, field) in [(ICON_SMALL, GCLP_HICONSM), (ICON_BIG, GCLP_HICON)] {
            let icon = SendMessageW(hwnd, WM_GETICON, Some(WPARAM(size as usize)), None);
            if icon.0 != 0 {
                SetClassLongPtrW(hwnd, field, icon.0);
            }
        }
    }
    Ok(())
}

impl ApplicationHandler for Player {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none()
            && let Err(error) = self.open(event_loop)
        {
            show_error(&format!("Не удалось открыть плеер SoundCloud.\n\n{error}"));
            event_loop.exit();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.view.take();
                self.window.take();
                event_loop.exit();
            }
            WindowEvent::Focused(focused) => {
                if let Some(view) = &self.view {
                    set_memory_target(view, focused);
                }
            }
            _ => {}
        }
    }
}

fn set_memory_target(view: &WebView, focused: bool) {
    let level = if focused {
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
    } else {
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
    };

    // The event loop owns the controller and invokes this on its COM apartment.
    let _ = unsafe {
        view.controller()
            .CoreWebView2()
            .and_then(|webview| webview.cast::<ICoreWebView2_19>())
            .and_then(|webview| webview.SetMemoryUsageTargetLevel(level))
    };
}

fn is_soundcloud_page(url: &str) -> bool {
    https_host_matches(url, |host| {
        host == "soundcloud.com" || host.ends_with(".soundcloud.com")
    })
}

fn is_sign_in_page(url: &str) -> bool {
    https_host_matches(url, |host| {
        matches!(
            host,
            "accounts.google.com" | "appleid.apple.com" | "www.facebook.com"
        )
    })
}

fn https_host_matches(url: &str, matches_host: impl FnOnce(&str) -> bool) -> bool {
    Url::parse(url).is_ok_and(|url| {
        url.scheme() == "https"
            && url.username().is_empty()
            && url.password().is_none()
            && url.host_str().is_some_and(matches_host)
    })
}

fn show_error(message: &str) {
    let message: Vec<u16> = message.encode_utf16().chain([0]).collect();
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(message.as_ptr()),
            w!("SoundCloud Lite"),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let app_data = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("Windows не указала папку LOCALAPPDATA."))?;
    let mut player = Player {
        view: None,
        window: None,
        context: WebContext::new(Some(app_data.join("SoundCloudLite").join("WebView2"))),
        widget_mode: env::args_os().skip(1).any(|arg| arg == "--widget"),
    };
    EventLoop::new()?.run_app(&mut player)?;
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        show_error(&format!("Не удалось запустить SoundCloud Lite.\n\n{error}"));
    }
}
