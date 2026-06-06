"use strict";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// --- DOM ---------------------------------------------------------------
const $ = (id) => document.getElementById(id);

const els = {
  navSettings: $("nav-settings"),
  viewMain: $("view-main"),
  viewSettings: $("view-settings"),
  // setup banner
  setup: $("setup"),
  setupBtn: $("setup-btn"),
  setupText: $("setup-text"),
  setupStatus: $("setup-status"),
  setupProgress: $("setup-progress"),
  setupBar: $("setup-bar"),
  // downloader
  url: $("url"),
  paste: $("paste"),
  modeVideo: $("mode-video"),
  modeAudio: $("mode-audio"),
  videoOpts: $("video-opts"),
  audioOpts: $("audio-opts"),
  quality: $("quality"),
  audioFormat: $("audio-format"),
  add: $("add"),
  info: $("info"),
  error: $("error"),
  preview: $("preview"),
  previewThumb: $("preview-thumb"),
  previewTitle: $("preview-title"),
  previewSub: $("preview-sub"),
  list: $("list"),
  empty: $("empty"),
  clearDone: $("clear-done"),
  // settings
  dlFolder: $("dl-folder"),
  changeFolder: $("change-folder"),
  openFolder: $("open-folder"),
  setMode: $("set-mode"),
  setQuality: $("set-quality"),
  setAudio: $("set-audio"),
  setConcurrent: $("set-concurrent"),
  setMeta: $("set-meta"),
  setThumb: $("set-thumb"),
  setOpen: $("set-open"),
  setTray: $("set-tray"),
  engineStatus: $("engine-status"),
  updateYtdlp: $("update-ytdlp"),
  reinstall: $("reinstall"),
  setYtdlpPath: $("set-ytdlp-path"),
  setFfmpegPath: $("set-ffmpeg-path"),
  settingsBack: $("settings-back"),
  settingsSave: $("settings-save"),
};

// --- State -------------------------------------------------------------
let settings = null;
let mode = "video";
const rows = new Map(); // job id -> { el, refs }

// --- Helpers -----------------------------------------------------------
function humanBytes(n) {
  if (!n || n <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; }
  return (i === 0 ? Math.round(n) : n.toFixed(1)) + " " + units[i];
}

function showError(msg) {
  els.error.textContent = msg;
  els.error.classList.remove("hidden");
}
function clearError() {
  els.error.textContent = "";
  els.error.classList.add("hidden");
}

function setMode(next) {
  mode = next;
  els.modeVideo.classList.toggle("active", next === "video");
  els.modeAudio.classList.toggle("active", next === "audio");
  els.videoOpts.classList.toggle("hidden", next !== "video");
  els.audioOpts.classList.toggle("hidden", next !== "audio");
}

// --- Settings ----------------------------------------------------------
async function loadSettings() {
  settings = await invoke("get_settings");
  setMode(settings.default_mode || "video");
  els.quality.value = settings.video_quality || "1080";
  els.audioFormat.value = settings.audio_format || "mp3";
}

function applySettingsToForm() {
  els.dlFolder.textContent = settings.download_dir || "";
  els.setMode.value = settings.default_mode || "video";
  els.setQuality.value = settings.video_quality || "1080";
  els.setAudio.value = settings.audio_format || "mp3";
  els.setConcurrent.value = String(settings.concurrent || 3);
  els.setMeta.checked = !!settings.embed_metadata;
  els.setThumb.checked = !!settings.embed_thumbnail;
  els.setOpen.checked = !!settings.open_when_done;
  els.setTray.checked = !!settings.close_to_tray;
  els.setYtdlpPath.value = settings.ytdlp_path || "";
  els.setFfmpegPath.value = settings.ffmpeg_path || "";
}

async function saveSettings() {
  const next = {
    ...settings,
    default_mode: els.setMode.value,
    video_quality: els.setQuality.value,
    audio_format: els.setAudio.value,
    concurrent: parseInt(els.setConcurrent.value, 10) || 3,
    embed_metadata: els.setMeta.checked,
    embed_thumbnail: els.setThumb.checked,
    open_when_done: els.setOpen.checked,
    close_to_tray: els.setTray.checked,
    ytdlp_path: els.setYtdlpPath.value.trim(),
    ffmpeg_path: els.setFfmpegPath.value.trim(),
  };
  await invoke("save_settings", { settings: next });
  settings = next;
  // Reflect on the main view.
  els.quality.value = settings.video_quality;
  els.audioFormat.value = settings.audio_format;
  showView("main");
}

function showView(which) {
  els.viewMain.classList.toggle("hidden", which !== "main");
  els.viewSettings.classList.toggle("hidden", which !== "settings");
  if (which === "settings") {
    applySettingsToForm();
    refreshEngineStatus();
  }
}

// --- Tool status / setup ----------------------------------------------
async function refreshEngineStatus() {
  try {
    const s = await invoke("check_binaries");
    const yt = s.ytdlp_ready ? `yt-dlp ${s.ytdlp_version} (${s.ytdlp_source})` : "yt-dlp missing";
    const ff = s.ffmpeg_ready ? `ffmpeg ready (${s.ffmpeg_source})` : "ffmpeg missing";
    els.engineStatus.textContent = `${yt} · ${ff}`;
    return s;
  } catch (e) {
    els.engineStatus.textContent = String(e);
    return null;
  }
}

async function checkSetup() {
  let status;
  try {
    status = await invoke("check_binaries");
  } catch {
    return;
  }
  const ready = status.ytdlp_ready && status.ffmpeg_ready;
  els.setup.classList.toggle("hidden", ready);
}

async function runSetup(force) {
  els.setupBtn.disabled = true;
  els.setupProgress.classList.remove("hidden");
  els.setupStatus.textContent = "Starting…";
  try {
    await invoke("ensure_binaries", { force });
    els.setupStatus.textContent = "Done";
    await checkSetup();
    await refreshEngineStatus();
  } catch (e) {
    els.setupStatus.textContent = "Failed: " + e;
  } finally {
    els.setupBtn.disabled = false;
    setTimeout(() => els.setupProgress.classList.add("hidden"), 1200);
  }
}

// --- Downloads ---------------------------------------------------------
async function addDownload() {
  clearError();
  const url = els.url.value.trim();
  if (!url) { showError("Paste a link first."); return; }

  const req = {
    url,
    mode,
    quality: mode === "video" ? els.quality.value : "",
    audio_format: mode === "audio" ? els.audioFormat.value : "",
  };

  els.add.disabled = true;
  try {
    await invoke("start_download", { req });
    els.url.value = "";
    els.preview.classList.add("hidden");
  } catch (e) {
    showError(String(e));
    if (String(e).toLowerCase().includes("set up")) els.setup.classList.remove("hidden");
  } finally {
    els.add.disabled = false;
  }
}

async function previewUrl() {
  clearError();
  const url = els.url.value.trim();
  if (!url) { showError("Paste a link first."); return; }
  els.info.disabled = true;
  els.info.textContent = "…";
  try {
    const info = await invoke("probe_url", { url });
    els.previewTitle.textContent = info.title || "(untitled)";
    const bits = [];
    if (info.uploader) bits.push(info.uploader);
    if (info.duration) bits.push(formatDuration(info.duration));
    if (info.heights && info.heights.length) bits.push(`up to ${info.heights[0]}p`);
    els.previewSub.textContent = bits.join(" · ");
    if (info.thumbnail) {
      els.previewThumb.src = info.thumbnail;
      els.previewThumb.style.visibility = "visible";
    } else {
      els.previewThumb.removeAttribute("src");
      els.previewThumb.style.visibility = "hidden";
    }
    els.preview.classList.remove("hidden");
  } catch (e) {
    showError(String(e));
  } finally {
    els.info.disabled = false;
    els.info.textContent = "Preview";
  }
}

function formatDuration(secs) {
  secs = Math.round(secs);
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  const pad = (n) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

// --- Job list rendering ------------------------------------------------
const STATUS_LABEL = {
  queued: "Queued",
  downloading: "Downloading",
  processing: "Processing",
  done: "Done",
  error: "Error",
  canceled: "Canceled",
};

function buildRow(job) {
  const el = document.createElement("div");
  el.className = "item";

  const top = document.createElement("div");
  top.className = "item-top";
  const title = document.createElement("div");
  title.className = "item-title";
  const badge = document.createElement("span");
  badge.className = "badge";
  top.append(title, badge);

  const progress = document.createElement("div");
  progress.className = "progress";
  const bar = document.createElement("div");
  bar.className = "progress-bar";
  progress.append(bar);

  const meta = document.createElement("div");
  meta.className = "item-meta";
  const stats = document.createElement("span");
  const spacer = document.createElement("span");
  spacer.className = "spacer";
  const actions = document.createElement("div");
  actions.className = "item-actions";
  meta.append(stats, spacer, actions);

  el.append(top, progress, meta);

  const refs = { el, title, badge, bar, stats, actions, progress };
  rows.set(job.id, refs);
  els.list.prepend(el);
  return refs;
}

function renderRow(job) {
  let refs = rows.get(job.id);
  if (!refs) refs = buildRow(job);

  refs.title.textContent = job.title || job.url;
  refs.title.title = job.title || job.url;
  refs.badge.textContent = STATUS_LABEL[job.status] || job.status;
  refs.badge.className = "badge " + job.status;

  const active = job.status === "downloading" || job.status === "processing" || job.status === "queued";
  refs.progress.style.display = active || job.status === "done" ? "" : "none";
  refs.bar.style.width = (job.percent || 0) + "%";

  const indeterminate =
    (job.status === "downloading" && (!job.percent || job.percent <= 0)) ||
    job.status === "processing" ||
    job.status === "queued";
  refs.el.classList.toggle("indeterminate", indeterminate);

  // Stats line
  if (job.status === "error") {
    refs.stats.textContent = job.error || "Failed";
  } else if (job.status === "downloading") {
    const bits = [];
    if (job.percent > 0) bits.push(Math.round(job.percent) + "%");
    if (job.speed) bits.push(job.speed);
    if (job.eta) bits.push("ETA " + job.eta);
    if (job.size) bits.push(job.size);
    refs.stats.textContent = bits.join(" · ") || "Starting…";
  } else if (job.status === "processing") {
    refs.stats.textContent = "Finishing up…";
  } else if (job.status === "done") {
    refs.stats.textContent = job.format + (job.size ? " · " + job.size : "");
  } else if (job.status === "queued") {
    refs.stats.textContent = "Waiting…";
  } else {
    refs.stats.textContent = job.format || "";
  }

  // Actions
  refs.actions.innerHTML = "";
  if (active) {
    refs.actions.append(makeBtn("Cancel", () => invoke("cancel_download", { id: job.id })));
  }
  if (job.status === "done") {
    if (job.filepath) {
      refs.actions.append(makeBtn("Show file", () => invoke("reveal_path", { path: job.filepath })));
    }
    refs.actions.append(makeBtn("Remove", () => removeRow(job.id)));
  }
  if (job.status === "error" || job.status === "canceled") {
    refs.actions.append(makeBtn("Remove", () => removeRow(job.id)));
  }

  updateEmptyState();
}

function makeBtn(label, onClick) {
  const b = document.createElement("button");
  b.className = "mini-btn";
  b.textContent = label;
  b.addEventListener("click", onClick);
  return b;
}

function removeRow(id) {
  const refs = rows.get(id);
  if (refs) { refs.el.remove(); rows.delete(id); }
  updateEmptyState();
}

function updateEmptyState() {
  const count = rows.size;
  els.empty.classList.toggle("hidden", count > 0);
  let hasDone = false;
  for (const [, refs] of rows) {
    if (refs.badge.classList.contains("done") ||
        refs.badge.classList.contains("error") ||
        refs.badge.classList.contains("canceled")) { hasDone = true; break; }
  }
  els.clearDone.classList.toggle("hidden", !hasDone);
}

function clearFinished() {
  for (const [id, refs] of [...rows]) {
    if (refs.badge.classList.contains("done") ||
        refs.badge.classList.contains("error") ||
        refs.badge.classList.contains("canceled")) {
      refs.el.remove();
      rows.delete(id);
    }
  }
  updateEmptyState();
}

// --- Events ------------------------------------------------------------
listen("job://update", (event) => renderRow(event.payload));

listen("setup://progress", (event) => {
  const p = event.payload;
  let text = p.message || "";
  if (p.total > 0) {
    text += `  ${humanBytes(p.downloaded)} / ${humanBytes(p.total)}`;
    els.setupBar.style.width = Math.min(100, (p.downloaded / p.total) * 100) + "%";
  } else if (p.downloaded > 0) {
    text += `  ${humanBytes(p.downloaded)}`;
  }
  els.setupStatus.textContent = text;
  if (p.phase === "done") els.setupBar.style.width = "100%";
});

// --- Wire up UI --------------------------------------------------------
els.modeVideo.addEventListener("click", () => setMode("video"));
els.modeAudio.addEventListener("click", () => setMode("audio"));
els.add.addEventListener("click", addDownload);
els.info.addEventListener("click", previewUrl);
els.url.addEventListener("keydown", (e) => { if (e.key === "Enter") addDownload(); });
els.clearDone.addEventListener("click", clearFinished);

els.paste.addEventListener("click", async () => {
  try {
    const text = await invoke("read_clipboard");
    if (text) { els.url.value = text.trim(); clearError(); }
  } catch (e) { showError(String(e)); }
});

els.navSettings.addEventListener("click", () => showView("settings"));
els.settingsBack.addEventListener("click", () => showView("main"));
els.settingsSave.addEventListener("click", saveSettings);
els.openFolder.addEventListener("click", () => invoke("open_downloads_folder"));
els.changeFolder.addEventListener("click", async () => {
  const dir = await invoke("pick_download_folder");
  if (dir) { settings.download_dir = dir; els.dlFolder.textContent = dir; }
});
els.setupBtn.addEventListener("click", () => runSetup(false));
els.reinstall.addEventListener("click", () => runSetup(true).then(refreshEngineStatus));
els.updateYtdlp.addEventListener("click", async () => {
  els.updateYtdlp.disabled = true;
  els.engineStatus.textContent = "Updating yt-dlp…";
  try {
    const v = await invoke("update_ytdlp");
    els.engineStatus.textContent = "Updated to yt-dlp " + v;
    await refreshEngineStatus();
  } catch (e) {
    els.engineStatus.textContent = "Update failed: " + e;
  } finally {
    els.updateYtdlp.disabled = false;
  }
});

// --- Boot --------------------------------------------------------------
(async function init() {
  await loadSettings();
  await checkSetup();
})();
