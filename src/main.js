const { invoke } = window.__TAURI__.core;
const { open } = window.__TAURI__.dialog;

const now = document.querySelector("#now");

document.querySelector("#open").addEventListener("click", async () => {
  const path = await open({
    multiple: false,
    filters: [{ name: "Аудио", extensions: ["mp3", "flac", "wav", "ogg"] }],
  });
  if (!path) return;
  await invoke("play", { path });
  now.textContent = path.split(/[\\/]/).pop();
});

document.querySelector("#pause").addEventListener("click", () => invoke("pause"));
document.querySelector("#resume").addEventListener("click", () => invoke("resume"));
document.querySelector("#stop").addEventListener("click", () => invoke("stop"));