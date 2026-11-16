// Вся логика плеера живёт в Rust. Здесь только кнопки и отрисовка.

const { invoke } = window.__TAURI__.core;
const { open } = window.__TAURI__.dialog;

const $ = (selector) => document.querySelector(selector);

const now = $("#now");
const seek = $("#seek");
const cur = $("#cur");
const dur = $("#dur");
const vol = $("#vol");
const toggle = $("#toggle");
const shuffleBtn = $("#shuffle");
const repeatBtn = $("#repeat");
const searchBox = $("#search");
const sortSel = $("#sort");
const dirBtn = $("#dir");
const count = $("#count");
const list = $("#list");
const spacer = $("#spacer");
const rows = $("#rows");

const ROW = 44; // высота строки в пикселях, как в styles.css
const REPEAT_NEXT = { off: "all", all: "one", one: "off" };
const REPEAT_TEXT = { off: "Повтор: выкл", all: "Повтор: все", one: "Повтор: трек" };

let tracks = []; // то, что сейчас показано: [{ title, artist, duration }]
let desc = false; // направление сортировки
let folderLoaded = false;
let last = {}; // последний статус из Rust
let dragging = false;
let drawnKey = "";

function fmt(s) {
  if (!isFinite(s)) return "0:00";
  s = Math.floor(s);
  const m = Math.floor(s / 60);
  return `${m}:${String(s % 60).padStart(2, "0")}`;
}

// ---------- Виртуальный список: рисуем только видимые строки ----------

function renderList(force = false) {
  const first = Math.max(0, Math.floor(list.scrollTop / ROW) - 5);
  const count = Math.ceil(list.clientHeight / ROW) + 10;
  const end = Math.min(tracks.length, first + count);

  const key = `${first}:${end}:${last.index}:${tracks.length}`;
  if (!force && key === drawnKey) return;
  drawnKey = key;

  rows.style.transform = `translateY(${first * ROW}px)`;

  const fragment = document.createDocumentFragment();
  for (let i = first; i < end; i++) {
    const t = tracks[i];
    const row = document.createElement("div");
    row.className = "row" + (i === last.index ? " current" : "");
    row.dataset.i = i;

    const num = document.createElement("span");
    num.className = "num";
    num.textContent = i + 1;

    const title = document.createElement("span");
    title.className = "title";
    title.textContent = t.title;

    const artist = document.createElement("span");
    artist.className = "artist";
    artist.textContent = t.artist;

    const time = document.createElement("span");
    time.className = "time";
    time.textContent = t.duration > 0 ? fmt(t.duration) : "";

    row.append(num, title, artist, time);
    fragment.append(row);
  }
  rows.replaceChildren(fragment);
}

list.addEventListener("scroll", () => renderList());
window.addEventListener("resize", () => renderList());

rows.addEventListener("click", (e) => {
  const row = e.target.closest(".row");
  if (row) invoke("play_index", { index: Number(row.dataset.i) });
});

// ---------- Обновление состояния ----------

async function refresh() {
  try {
    last = await invoke("get_status");
  } catch (e) {
    return;
  }

  const hasDuration = last.duration != null && last.duration > 0;
  seek.disabled = !last.has_track || !hasDuration;
  dur.textContent = hasDuration ? fmt(last.duration) : "--:--";

  if (hasDuration) seek.max = last.duration;
  if (!dragging) {
    seek.value = last.position;
    cur.textContent = fmt(last.position);
  }

  toggle.textContent = last.playing ? "Пауза" : last.has_track && !last.finished ? "Продолжить" : "Играть";

  shuffleBtn.classList.toggle("on", !!last.shuffle);
  repeatBtn.classList.toggle("on", last.repeat !== "off");
  repeatBtn.textContent = REPEAT_TEXT[last.repeat] ?? REPEAT_TEXT.off;

  const t = last.has_track && last.index != null ? tracks[last.index] : null;
  if (t) now.textContent = t.artist ? `${t.artist} — ${t.title}` : t.title;

  renderList();
}

setInterval(refresh, 250);

// ---------- Поиск и сортировка (считает Rust) ----------

function viewArgs() {
  return { query: searchBox.value, sort: sortSel.value, desc };
}

function showTracks(list_) {
  tracks = list_;
  spacer.style.height = `${tracks.length * ROW}px`;
  count.textContent = tracks.length ? `${tracks.length} шт.` : "";
  renderList(true);
}

async function applyView() {
  if (!folderLoaded) return;
  showTracks(await invoke("set_view", viewArgs()));
}

let searchTimer = null;
searchBox.addEventListener("input", () => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(applyView, 150); // ждём, пока человек допечатает
});
sortSel.addEventListener("change", applyView);
dirBtn.addEventListener("click", () => {
  desc = !desc;
  dirBtn.textContent = desc ? "↓" : "↑";
  applyView();
});

// ---------- Кнопки ----------

$("#folder").addEventListener("click", async () => {
  const dir = await open({ directory: true, multiple: false });
  if (!dir) return;

  now.textContent = "Сканирую папку…";
  try {
    showTracks(await invoke("scan_folder", { path: dir, ...viewArgs() }));
  } catch (e) {
    now.textContent = "Не удалось прочитать папку";
    return;
  }

  folderLoaded = true;
  list.scrollTop = 0;
  now.textContent = tracks.length ? "Выбери трек" : "Ничего не найдено";
});

toggle.addEventListener("click", async () => {
  if (!tracks.length) return;
  if (!last.has_track || last.finished) await invoke("play_index", { index: last.index ?? 0 });
  else if (last.playing) await invoke("pause");
  else await invoke("resume");
});

$("#prev").addEventListener("click", () => invoke("prev"));
$("#next").addEventListener("click", () => invoke("next"));
$("#stop").addEventListener("click", () => invoke("stop"));

shuffleBtn.addEventListener("click", () => invoke("set_shuffle", { on: !last.shuffle }));
repeatBtn.addEventListener("click", () => invoke("set_repeat", { mode: REPEAT_NEXT[last.repeat] ?? "all" }));

seek.addEventListener("input", () => {
  dragging = true;
  cur.textContent = fmt(Number(seek.value));
});
seek.addEventListener("change", async () => {
  await invoke("seek", { position: Number(seek.value) });
  dragging = false;
});

vol.addEventListener("input", () => {
  invoke("set_volume", { volume: Number(vol.value) / 100 });
});