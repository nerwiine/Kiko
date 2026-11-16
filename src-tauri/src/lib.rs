use std::{
    fs::File,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Duration,
};

use rodio::{Decoder, OutputStreamBuilder, Sink, Source};
use serde::Serialize;

enum Cmd {
    Play(String),
    Pause,
    Resume,
    Stop,
    Seek(f64),
    Volume(f32),
}

#[derive(Clone, Default, Serialize)]
struct Status {
    has_track: bool,
    playing: bool,
    finished: bool,
    position: f64,
    duration: Option<f64>,
}

struct AudioState {
    tx: Mutex<mpsc::Sender<Cmd>>,
    status: Arc<Mutex<Status>>,
}

fn send(state: &tauri::State<AudioState>, cmd: Cmd) {
    let _ = state.tx.lock().unwrap().send(cmd);
}

#[tauri::command]
fn play(path: String, state: tauri::State<AudioState>) {
    send(&state, Cmd::Play(path));
}

#[tauri::command]
fn pause(state: tauri::State<AudioState>) {
    send(&state, Cmd::Pause);
}

#[tauri::command]
fn resume(state: tauri::State<AudioState>) {
    send(&state, Cmd::Resume);
}

#[tauri::command]
fn stop(state: tauri::State<AudioState>) {
    send(&state, Cmd::Stop);
}

#[tauri::command]
fn seek(position: f64, state: tauri::State<AudioState>) {
    send(&state, Cmd::Seek(position));
}

#[tauri::command]
fn set_volume(volume: f32, state: tauri::State<AudioState>) {
    send(&state, Cmd::Volume(volume));
}

#[tauri::command]
fn get_status(state: tauri::State<AudioState>) -> Status {
    state.status.lock().unwrap().clone()
}

fn audio_thread(rx: mpsc::Receiver<Cmd>, status: Arc<Mutex<Status>>) {
    let stream = OutputStreamBuilder::open_default_stream().expect("нет устройства вывода звука");
    let mut sink: Option<Sink> = None;
    let mut duration: Option<f64> = None;
    let mut volume: f32 = 1.0;

    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(cmd) => match cmd {
                Cmd::Play(path) => {
                    sink = None;
                    duration = None;
                    if let Ok(file) = File::open(&path) {
                        if let Ok(source) = Decoder::try_from(file) {
                            duration = source.total_duration().map(|d| d.as_secs_f64());
                            let new_sink = Sink::connect_new(stream.mixer());
                            new_sink.set_volume(volume);
                            new_sink.append(source);
                            sink = Some(new_sink);
                        }
                    }
                }
                Cmd::Pause => {
                    if let Some(s) = &sink {
                        s.pause();
                    }
                }
                Cmd::Resume => {
                    if let Some(s) = &sink {
                        s.play();
                    }
                }
                Cmd::Stop => {
                    sink = None;
                    duration = None;
                }
                Cmd::Seek(secs) => {
                    if let Some(s) = &sink {
                        let _ = s.try_seek(Duration::from_secs_f64(secs.max(0.0)));
                    }
                }
                Cmd::Volume(v) => {
                    volume = v.clamp(0.0, 1.0);
                    if let Some(s) = &sink {
                        s.set_volume(volume);
                    }
                }
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        let mut st = status.lock().unwrap();
        match &sink {
            Some(s) => {
                let finished = s.empty();
                st.has_track = true;
                st.finished = finished;
                st.playing = !finished && !s.is_paused();
                st.duration = duration;
                if finished {
                    if let Some(d) = duration {
                        st.position = d;
                    }
                } else {
                    st.position = s.get_pos().as_secs_f64();
                }
            }
            None => *st = Status::default(),
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (tx, rx) = mpsc::channel();
    let status = Arc::new(Mutex::new(Status::default()));
    let status_for_thread = status.clone();
    thread::spawn(move || audio_thread(rx, status_for_thread));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AudioState {
            tx: Mutex::new(tx),
            status,
        })
        .invoke_handler(tauri::generate_handler![
            play, pause, resume, stop, seek, set_volume, get_status
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}