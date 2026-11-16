use std::{
    fs::File,
    path::Path,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use lofty::prelude::*;
use rodio::{Decoder, OutputStream, OutputStreamBuilder, Sink, Source};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

// ---------- Типы ----------

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Repeat {
    #[default]
    Off, // не повторять
    All, // повторять весь список
    One, // повторять один трек
}

// Как сортировать список
#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Sort {
    #[default]
    Folder, // как лежат файлы в папке
    Title,
    Artist,
    Duration,
}

// Команды, которые поток звука получает из интерфейса
enum Cmd {
    Clear, // новая папка: забыть всё
    SetPlaylist(Vec<String>),
    PlayIndex(usize),
    Next,
    Prev,
    Pause,
    Resume,
    Stop,
    Seek(f64),
    Volume(f32),
    Shuffle(bool),
    SetRepeat(Repeat),
}

// То, что интерфейс узнаёт о плеере
#[derive(Clone, Default, Serialize)]
struct Status {
    has_track: bool,
    playing: bool,
    finished: bool,
    position: f64,
    duration: Option<f64>,
    index: Option<usize>, // номер играющего трека в списке
    shuffle: bool,
    repeat: Repeat,
}

// Информация о треке для отображения в списке
#[derive(Serialize)]
struct TrackInfo {
    title: String,
    artist: String,
    duration: f64,
}

// Трек в библиотеке (всё, что нашли в папке)
struct Track {
    path: String,
    title: String,
    artist: String,
    duration: f64,
    // то же самое в нижнем регистре: для поиска и сортировки без лишней работы
    title_lc: String,
    artist_lc: String,
    name_lc: String,
}

struct Library {
    tracks: Mutex<Vec<Track>>,
}

struct AudioState {
    tx: Mutex<mpsc::Sender<Cmd>>,
    status: Arc<Mutex<Status>>,
}

fn send(state: &tauri::State<AudioState>, cmd: Cmd) {
    let _ = state.tx.lock().unwrap().send(cmd);
}

// ---------- Команды для интерфейса ----------

#[tauri::command]
fn play_index(index: usize, state: tauri::State<AudioState>) {
    send(&state, Cmd::PlayIndex(index));
}

#[tauri::command]
fn next(state: tauri::State<AudioState>) {
    send(&state, Cmd::Next);
}

#[tauri::command]
fn prev(state: tauri::State<AudioState>) {
    send(&state, Cmd::Prev);
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
fn set_shuffle(on: bool, state: tauri::State<AudioState>) {
    send(&state, Cmd::Shuffle(on));
}

#[tauri::command]
fn set_repeat(mode: Repeat, state: tauri::State<AudioState>) {
    send(&state, Cmd::SetRepeat(mode));
}

#[tauri::command]
fn get_status(state: tauri::State<AudioState>) -> Status {
    state.status.lock().unwrap().clone()
}

// Строит список для показа: фильтр по поиску + сортировка.
// Этот же список становится очередью воспроизведения.
fn apply_view(
    lib: &Library,
    state: &tauri::State<AudioState>,
    query: &str,
    sort: Sort,
    desc: bool,
) -> Vec<TrackInfo> {
    let tracks = lib.tracks.lock().unwrap();
    let q = query.trim().to_lowercase();

    let mut view: Vec<&Track> = tracks
        .iter()
        .filter(|t| {
            q.is_empty()
                || t.title_lc.contains(&q)
                || t.artist_lc.contains(&q)
                || t.name_lc.contains(&q)
        })
        .collect();

    match sort {
        Sort::Folder => {}
        Sort::Title => view.sort_by(|a, b| a.title_lc.cmp(&b.title_lc)),
        Sort::Artist => view.sort_by(|a, b| {
            a.artist_lc
                .cmp(&b.artist_lc)
                .then_with(|| a.title_lc.cmp(&b.title_lc))
        }),
        Sort::Duration => view.sort_by(|a, b| a.duration.total_cmp(&b.duration)),
    }
    if desc {
        view.reverse();
    }

    send(
        state,
        Cmd::SetPlaylist(view.iter().map(|t| t.path.clone()).collect()),
    );

    view.iter()
        .map(|t| TrackInfo {
            title: t.title.clone(),
            artist: t.artist.clone(),
            duration: t.duration,
        })
        .collect()
}

// Сканирует папку и возвращает список для показа
#[tauri::command]
async fn scan_folder(
    path: String,
    query: String,
    sort: Sort,
    desc: bool,
    state: tauri::State<'_, AudioState>,
    lib: tauri::State<'_, Library>,
) -> Result<Vec<TrackInfo>, String> {
    let tracks = tauri::async_runtime::spawn_blocking(move || scan(&path))
        .await
        .map_err(|e| e.to_string())?;
    *lib.tracks.lock().unwrap() = tracks;
    send(&state, Cmd::Clear);
    Ok(apply_view(&lib, &state, &query, sort, desc))
}

// Поиск или смена сортировки: список меняется, музыка не прерывается
#[tauri::command]
fn set_view(
    query: String,
    sort: Sort,
    desc: bool,
    state: tauri::State<AudioState>,
    lib: tauri::State<Library>,
) -> Vec<TrackInfo> {
    apply_view(&lib, &state, &query, sort, desc)
}

// ---------- Сканирование папки и теги ----------

const EXTENSIONS: [&str; 4] = ["mp3", "flac", "wav", "ogg"];

fn scan(root: &str) -> Vec<Track> {
    let mut files: Vec<String> = WalkDir::new(root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|x| x.to_str())
                .map(|x| EXTENSIONS.contains(&x.to_lowercase().as_str()))
                .unwrap_or(false)
        })
        .map(|e| e.path().to_string_lossy().into_owned())
        .collect();

    files.sort_by_key(|p| p.to_lowercase());
    files.iter().map(|p| read_track(p)).collect()
}

fn read_track(path: &str) -> Track {
    let fallback = Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    let (title, artist, duration) = match lofty::read_from_path(path) {
        Ok(tagged) => {
            let duration = tagged.properties().duration().as_secs_f64();
            let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
            let title = tag
                .and_then(|t| t.title().map(|s| s.to_string()))
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| fallback.clone());
            let artist = tag
                .and_then(|t| t.artist().map(|s| s.to_string()))
                .unwrap_or_default();
            (title, artist, duration)
        }
        Err(_) => (fallback.clone(), String::new(), 0.0),
    };

    Track {
        path: path.to_string(),
        title_lc: title.to_lowercase(),
        artist_lc: artist.to_lowercase(),
        name_lc: fallback.to_lowercase(),
        title,
        artist,
        duration,
    }
}

// ---------- Плеер (живёт в отдельном потоке) ----------

struct Player {
    stream: OutputStream,
    sink: Option<Sink>,
    duration: Option<f64>,
    volume: f32,

    tracks: Vec<String>, // пути к файлам, в порядке списка
    order: Vec<usize>,   // порядок воспроизведения (номера из tracks)
    pos: Option<usize>,  // где мы в order
    shuffle: bool,
    repeat: Repeat,
    ended: bool, // автопереход на этом треке уже обработан
    rng: u64,
}

impl Player {
    fn new() -> Self {
        let stream =
            OutputStreamBuilder::open_default_stream().expect("нет устройства вывода звука");
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
        Player {
            stream,
            sink: None,
            duration: None,
            volume: 1.0,
            tracks: Vec::new(),
            order: Vec::new(),
            pos: None,
            shuffle: false,
            repeat: Repeat::Off,
            ended: false,
            rng: seed | 1,
        }
    }

    // Простой генератор случайных чисел (xorshift), без внешних библиотек
    fn next_rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    // Пересобирает порядок воспроизведения. Текущий трек остаётся текущим.
    fn rebuild_order(&mut self) {
        let n = self.tracks.len();
        let current = self.pos.and_then(|p| self.order.get(p).copied());
        let mut order: Vec<usize> = (0..n).collect();

        if self.shuffle {
            for i in (1..n).rev() {
                let j = (self.next_rand() % (i as u64 + 1)) as usize;
                order.swap(i, j);
            }
            // текущий трек ставим в начало, чтобы дальше шли остальные
            if let Some(c) = current {
                if let Some(k) = order.iter().position(|&x| x == c) {
                    order.swap(0, k);
                }
            }
        }

        self.pos = current.and_then(|c| order.iter().position(|&x| x == c));
        self.order = order;
    }

    fn stop(&mut self) {
        self.sink = None;
        self.duration = None;
        self.ended = false;
    }

    // Запускает трек под номером pos в порядке воспроизведения.
    // Возвращает false, если файл не открылся.
    fn play_pos(&mut self, pos: usize) -> bool {
        self.sink = None;
        self.duration = None;
        self.ended = false;

        let Some(&idx) = self.order.get(pos) else {
            return false;
        };
        self.pos = Some(pos);

        let Some(path) = self.tracks.get(idx).cloned() else {
            return false;
        };
        let Ok(file) = File::open(&path) else {
            return false;
        };
        let Ok(source) = Decoder::try_from(file) else {
            return false;
        };

        self.duration = source.total_duration().map(|d| d.as_secs_f64());
        let sink = Sink::connect_new(self.stream.mixer());
        sink.set_volume(self.volume);
        sink.append(source);
        self.sink = Some(sink);
        true
    }

    // auto = true, когда трек доиграл сам; false, когда нажали «Следующий»
    fn next(&mut self, auto: bool) {
        let n = self.order.len();
        if n == 0 {
            return;
        }

        if auto && self.repeat == Repeat::One {
            if let Some(p) = self.pos {
                self.play_pos(p);
            }
            return;
        }

        let mut p = self.pos.map_or(0, |p| p + 1);
        for _ in 0..n {
            if p >= n {
                // дошли до конца списка
                if auto && self.repeat == Repeat::Off {
                    return; // просто останавливаемся (finished = true)
                }
                p = 0;
            }
            if self.play_pos(p) {
                return;
            }
            p += 1; // файл не открылся, пробуем следующий
        }
        self.stop();
    }

    fn prev(&mut self) {
        let n = self.order.len();
        if n == 0 {
            return;
        }

        // если трек играет дольше 3 секунд, «Предыдущий» начинает его заново
        let played = self
            .sink
            .as_ref()
            .map(|s| s.get_pos().as_secs_f64())
            .unwrap_or(0.0);
        if played > 3.0 {
            if let Some(p) = self.pos {
                self.play_pos(p);
                return;
            }
        }

        let mut p = match self.pos {
            Some(p) if p > 0 => p - 1,
            _ => n - 1,
        };
        for _ in 0..n {
            if self.play_pos(p) {
                return;
            }
            p = if p == 0 { n - 1 } else { p - 1 };
        }
        self.stop();
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Clear => {
                self.stop();
                self.tracks.clear();
                self.order.clear();
                self.pos = None;
            }
            Cmd::SetPlaylist(tracks) => {
                // музыка не прерывается: запоминаем играющий трек и ищем его в новом списке
                let current = self
                    .pos
                    .and_then(|p| self.order.get(p))
                    .and_then(|&i| self.tracks.get(i))
                    .cloned();
                self.tracks = tracks;
                self.order = (0..self.tracks.len()).collect();
                self.pos = current.and_then(|c| self.tracks.iter().position(|t| *t == c));
                self.rebuild_order();
            }
            Cmd::PlayIndex(i) => {
                if i >= self.tracks.len() {
                    return;
                }
                self.pos = self.order.iter().position(|&x| x == i);
                // при случайном порядке начинаем новый круг с выбранного трека
                self.rebuild_order();
                if let Some(p) = self.pos {
                    self.play_pos(p);
                }
            }
            Cmd::Next => self.next(false),
            Cmd::Prev => self.prev(),
            Cmd::Pause => {
                if let Some(s) = &self.sink {
                    s.pause();
                }
            }
            Cmd::Resume => {
                if let Some(s) = &self.sink {
                    s.play();
                }
            }
            Cmd::Stop => self.stop(),
            Cmd::Seek(secs) => {
                if let Some(s) = &self.sink {
                    let _ = s.try_seek(Duration::from_secs_f64(secs.max(0.0)));
                }
            }
            Cmd::Volume(v) => {
                self.volume = v.clamp(0.0, 1.0);
                if let Some(s) = &self.sink {
                    s.set_volume(self.volume);
                }
            }
            Cmd::Shuffle(on) => {
                self.shuffle = on;
                self.rebuild_order();
            }
            Cmd::SetRepeat(mode) => self.repeat = mode,
        }
    }

    // Вызывается каждые 100 мс: проверяет, не доиграл ли трек
    fn tick(&mut self) {
        let finished = self.sink.as_ref().map_or(false, |s| s.empty());
        if finished && !self.ended {
            self.ended = true;
            self.next(true);
        }
    }

    fn write_status(&self, status: &Mutex<Status>) {
        let mut st = status.lock().unwrap();
        *st = Status::default();
        st.shuffle = self.shuffle;
        st.repeat = self.repeat;
        st.index = self.pos.and_then(|p| self.order.get(p).copied());

        if let Some(s) = &self.sink {
            let finished = s.empty();
            st.has_track = true;
            st.finished = finished;
            st.playing = !finished && !s.is_paused();
            st.duration = self.duration;
            if finished {
                if let Some(d) = self.duration {
                    st.position = d;
                }
            } else {
                st.position = s.get_pos().as_secs_f64();
            }
        }
    }
}

fn audio_thread(rx: mpsc::Receiver<Cmd>, status: Arc<Mutex<Status>>) {
    let mut player = Player::new();

    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(cmd) => player.handle(cmd),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        player.tick();
        player.write_status(&status);
    }
}

// ---------- Запуск приложения ----------

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
        .manage(Library {
            tracks: Mutex::new(Vec::new()),
        })
        .invoke_handler(tauri::generate_handler![
            play_index,
            next,
            prev,
            pause,
            resume,
            stop,
            seek,
            set_volume,
            set_shuffle,
            set_repeat,
            get_status,
            scan_folder,
            set_view
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}