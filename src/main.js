const { invoke } = window.__TAURI__.core;
const { open } = window.__TAURI__.dialog;

const now = document.querySelector("#now");
const seek = document.querySelector("#seek");
const cur = document.querySelector("#cur");
const dur = document.querySelector("#dur");
const vol = document.querySelector("#vol");
const toggle = document.querySelector("#toggle");

let currentPath = null;
let last = {};
let dragging = false;

function fmt(s) {
  if (!isFinite(s)) return "0:00";
  s = Math.floor(s);
  const m = Math.floor(s / 60);
  return `${m}:${String(s % 60).padStart(2, "0")}`;
}

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
}

setInterval(refresh, 250);

document.querySelector("#open").addEventListener("click", async () => {
  const path = await open({
    multiple: false,
    filters: [{ name: "Аудио", extensions: ["mp3", "flac", "wav", "ogg"] }],
  });
  if (!path) return;
  currentPath = path;
  await invoke("play", { path });
  now.textContent = path.split(/[\\/]/).pop();
});

toggle.addEventListener("click", async () => {
  if (!currentPath) return;
  if (!last.has_track || last.finished) await invoke("play", { path: currentPath });
  else if (last.playing) await invoke("pause");
  else await invoke("resume");
});

document.querySelector("#stop").addEventListener("click", () => invoke("stop"));

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