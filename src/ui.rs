slint::slint! {
    #[edition = "1.19"]
    import { LineEdit, ListView, Palette } from "std-widgets.slint";

    component FlatButton inherits Rectangle {
        in property <string> text;
        in property <bool> enabled: true;
        in property <bool> primary;
        callback clicked();
        min-width: max(28px, label.preferred-width + 20px);
        min-height: 31px;
        border-radius: 5px;
        border-width: 1px;
        border-color: keyboard.has-focus ? white : #3a3a3a;
        background: !root.enabled ? #121212
            : root.primary ? (pointer.has-hover ? #dadada : white)
            : pointer.has-hover ? #282828 : #141414;
        forward-focus: keyboard;
        accessible-role: button;
        accessible-label: root.text;
        accessible-enabled: root.enabled;
        accessible-action-default => { if root.enabled { root.clicked(); } }
        label := Text {
            text: root.text;
            color: !root.enabled ? #8a8a8a : root.primary ? black : white;
            horizontal-alignment: center;
            vertical-alignment: center;
        }
        keyboard := FocusScope {
            enabled: root.enabled;
            key-pressed(event) => {
                if event.text == Key.Return || event.text == Key.Space {
                    root.clicked();
                    return accept;
                }
                reject
            }
        }
        pointer := TouchArea {
            enabled: root.enabled;
            mouse-cursor: MouseCursor.pointer;
            clicked => { keyboard.focus(); root.clicked(); }
        }
    }

    export struct TrackRow {
        title: string,
        artist: string,
        duration: string,
    }

    export component PlayerWindow inherits Window {
        init => { Palette.color-scheme = ColorScheme.dark; }
        title: "SoundCloud Lite";
        background: #000000;
        preferred-width: 720px;
        preferred-height: 540px;
        min-width: 510px;
        min-height: 410px;
        default-font-family: "Segoe UI";
        default-font-size: 14px;

        in property <[TrackRow]> tracks;
        in property <bool> signed-in;
        in property <bool> busy;
        in property <string> status;
        in property <string> user-name;
        in property <string> now-title;
        in property <string> now-artist;
        in property <bool> playing;
        in property <bool> can-pause;
        in property <int> selected: -1;
        property <int> volume: 55;

        callback sign-in();
        callback sign-out();
        callback search(string);
        callback play-track(int);
        callback toggle-playback();
        callback volume-changed(float);
        callback open-track();
        callback network-ready();

        VerticalLayout {
            padding: 22px;
            spacing: 14px;

            HorizontalLayout {
                height: 36px;
                spacing: 12px;
                Text {
                    text: "SoundCloud Lite";
                    font-size: 23px;
                    font-weight: 700;
                    color: white;
                    vertical-alignment: center;
                    horizontal-stretch: 1;
                }
                if root.user-name != "": Text {
                    text: root.user-name;
                    color: #bcbcbc;
                    overflow: elide;
                    max-width: 150px;
                    vertical-alignment: center;
                }
                FlatButton {
                    text: root.signed-in ? "Выйти" : "Войти";
                    primary: !root.signed-in;
                    enabled: !root.busy;
                    clicked => {
                        if root.signed-in { root.sign-out(); } else { root.sign-in(); }
                    }
                }
            }

            HorizontalLayout {
                spacing: 8px;
                height: 36px;
                query := LineEdit {
                    placeholder-text: "Название или исполнитель";
                    horizontal-stretch: 1;
                    enabled: root.signed-in;
                    accepted(input) => { if !root.busy { root.search(input); } }
                }
                FlatButton {
                    text: "Найти";
                    enabled: root.signed-in && !root.busy && query.text != "";
                    clicked => { root.search(query.text); }
                }
            }

            if root.status != "": Text {
                text: root.status;
                color: #d6d6d6;
                wrap: word-wrap;
            }

            Rectangle {
                background: #080808;
                border-color: #303030;
                border-width: 1px;
                border-radius: 8px;
                vertical-stretch: 1;
                clip: true;

                ListView {
                    x: 1px;
                    y: 1px;
                    width: parent.width - 2px;
                    height: parent.height - 2px;

                    for track[index] in root.tracks: Rectangle {
                        height: 54px;
                        background: root.selected == index ? #242424
                            : row-pointer.has-hover ? #181818 : #080808;
                        HorizontalLayout {
                            padding-left: 12px;
                            padding-right: 12px;
                            spacing: 12px;
                            VerticalLayout {
                                alignment: center;
                                spacing: 2px;
                                horizontal-stretch: 1;
                                Text { text: track.title; color: white; font-weight: 600; overflow: elide; }
                                Text { text: track.artist; color: #bcbcbc; font-size: 12px; overflow: elide; }
                            }
                            Text { text: track.duration; color: #bcbcbc; vertical-alignment: center; }
                        }
                        row-pointer := TouchArea {
                            mouse-cursor: pointer;
                            enabled: !root.busy;
                            clicked => { root.play-track(index); }
                        }
                    }
                }
            }

            HorizontalLayout {
                height: 36px;
                spacing: 10px;
                VerticalLayout {
                    alignment: center;
                    horizontal-stretch: 1;
                    Text {
                        text: root.now-title == "" ? "Трек не выбран" : root.now-title;
                        color: white;
                        font-weight: 600;
                        overflow: elide;
                    }
                    if root.now-artist != "": Text {
                        text: root.now-artist;
                        color: #bcbcbc;
                        font-size: 12px;
                        overflow: elide;
                    }
                }
                if root.now-title != "": FlatButton {
                    text: "SoundCloud ↗";
                    clicked => { root.open-track(); }
                }
                FlatButton {
                    text: root.playing ? "Пауза" : "Играть";
                    primary: true;
                    enabled: root.can-pause;
                    clicked => { root.toggle-playback(); }
                }
                FlatButton {
                    text: "−";
                    accessible-label: "Уменьшить громкость";
                    enabled: root.volume > 0;
                    clicked => {
                        root.volume = max(0, root.volume - 5);
                        root.volume-changed(root.volume / 100);
                    }
                }
                Text {
                    text: root.volume + "%";
                    min-width: 38px;
                    color: #d6d6d6;
                    horizontal-alignment: center;
                    vertical-alignment: center;
                }
                FlatButton {
                    text: "+";
                    accessible-label: "Увеличить громкость";
                    enabled: root.volume < 100;
                    clicked => {
                        root.volume = min(100, root.volume + 5);
                        root.volume-changed(root.volume / 100);
                    }
                }
            }
        }
    }
}
