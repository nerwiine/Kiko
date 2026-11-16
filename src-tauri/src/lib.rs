use std::{fs::File, io::BufReader, sync::{mpsc, Mutex}, thread};
use rodio::{Decoder, OutputStreamBuilder, Sink};

enum Cmd {
    Play(String),
    Pause,
    Resume,
    Stop,
}

struct AudioState(Mutex<mpsc::Sender<Cmd>>);

fn send(state: &tauri::State<AudioState>, cmd: Cmd) {
    let _ = state.0.lock().unwrap().send(cmd);
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

fn audio_thread(rx: mpsc::Receiver<Cmd>) {
    let stream = OutputStreamBuilder::open_default_stream().expect("нет устройства вывода звука");
    let mut sink: Option<Sink> = None;

    for cmd in rx {
        match cmd {
            Cmd::Play(path) => {
                sink = None; // старый трек останавливается
                if let Ok(file) = File::open(&path) {
                    if let Ok(source) = Decoder::new(BufReader::new(file)) {
                        let new_sink = Sink::connect_new(stream.mixer());
                        new_sink.append(source);
                        sink = Some(new_sink);
                    }
                }
            }
            Cmd::Pause => {
                if let Some(s) = &sink { s.pause(); }
            }
            Cmd::Resume => {
                if let Some(s) = &sink { s.play(); }
            }
            Cmd::Stop => {
                sink = None;
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || audio_thread(rx));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AudioState(Mutex::new(tx)))
        .invoke_handler(tauri::generate_handler![play, pause, resume, stop])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}