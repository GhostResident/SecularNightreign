const $ = id => document.getElementById(id);
const tauri = window.__TAURI__;
const invoke = (command, args) => tauri.core.invoke(command, args);
const state = { game: null, version: null, rows: [], mod: null, busy: false };
const chain = [...$("chain").children];
const target = chain.at(-1).dataset.version;
const mirrorLabel = { multipart: "Зеркало · EXE + части BIN", archive: "Зеркало · архив ZIP/RAR" };
const phaseLabel = {
  download: "Загрузка", assemble: "Сборка BIN", extract: "Распаковка",
  apply: "Копирование", install: "Установка", ready: "Готов", done: "Установлен",
};

function status(message, tone = "") {
  $("status").textContent = message;
  $("status").title = message;
  $("status-dot").dataset.tone = tone;
}

function log(message, tone = "") {
  const line = document.createElement("li");
  line.dataset.tone = tone;
  const clock = document.createElement("time");
  clock.textContent = new Date().toLocaleTimeString("ru-RU");
  const text = document.createElement("span");
  text.textContent = message;
  line.append(clock, text);
  const list = $("log");
  list.append(line);
  if (list.children.length > 200) list.firstElementChild.remove();
  $("log-placeholder").hidden = true;
  list.scrollTop = list.scrollHeight;
}

function badge(element, message, tone = "") {
  element.textContent = message;
  element.dataset.tone = tone;
  element.hidden = !message;
}

function gameMessage(message = "", tone = "") {
  const element = $("game-message");
  element.hidden = !message;
  element.textContent = message;
  element.dataset.tone = tone;
}

function result(message = "", tone = "ok") {
  $("result").textContent = message;
  $("result").dataset.tone = tone;
  $("result").hidden = !message;
}

function renderChain() {
  const position = chain.findIndex(node => node.dataset.version === state.version);
  chain.forEach((node, index) => {
    const current = index === position;
    node.classList.toggle("current", current);
    node.classList.toggle("past", index < position);
    node.querySelector("small").textContent = current ? "Текущая" : index === chain.length - 1 ? "Хотфикс" : "";
    if (current) node.setAttribute("aria-current", "step");
    else node.removeAttribute("aria-current");
  });
}

const readyPackage = row => row.mode === "local" ? row.local : row.download;
const remainingRows = () => state.rows.filter(row => !row.done);
const pending = () => remainingRows().filter(row => row.mode === "download" && row.mirror && !row.download);

function sync() {
  document.body.classList.toggle("has-game", !!state.game);
  $("game-path").disabled = state.busy;
  $("game-browse").disabled = state.busy;
  $("verify").disabled = state.busy || !$("game-path").value.trim();
  $("manual-version").disabled = state.busy;
  $("open-game").hidden = !state.game;
  $("open-game").disabled = state.busy;
  const left = remainingRows();
  $("patch-actions").hidden = !left.length;
  $("install").disabled = state.busy || !left.length || !left.every(readyPackage);
  $("install").textContent = left.length === 1 && left[0].step.direct ? "Применить хотфикс" : "Установить патчи";
  $("download-all").hidden = pending().length < 2;
  $("download-all").disabled = state.busy;
  state.rows.forEach(renderRow);
  renderChain();
  renderMod();
}

async function run(action, onError = null) {
  if (state.busy) return;
  state.busy = true;
  sync();
  try {
    await action();
  } catch (error) {
    const message = typeof error === "string" ? error : error.message ?? String(error);
    log(message, "error");
    status(message, "error");
    onError?.(message);
  } finally {
    state.busy = false;
    sync();
  }
}

function resetGame() {
  state.game = null;
  state.version = null;
  if (state.mod) {
    state.mod = { ...state.mod, installed: null, state: "unselected", needsInstall: !!state.mod.latest, newerInstalled: false };
  }
  modNote();
  $("mod-progress").hidden = true;
  $("manual").hidden = true;
  $("manual-version").value = "";
  badge($("game-badge"), "");
  gameMessage();
  result();
  renderPlan([]);
}

function renderPlan(steps, keep = false) {
  const previous = keep ? state.rows : [];
  state.rows = steps.map(step => {
    const old = previous.find(row => row.step.from === step.from);
    const mirror = step.mirrors.includes(old?.mirror) ? old.mirror : step.mirrors[0];
    return makeRow(step, {
      mode: old?.mode ?? (mirror ? "download" : "local"),
      mirror,
      localText: old?.localText ?? "",
      local: old?.local ?? null,
      download: old?.mirror === mirror ? old?.download : null,
    });
  });
  $("patch-list").replaceChildren(...state.rows.map(row => row.el));
  const caption = $("patch-caption");
  caption.textContent = steps.length
    ? (steps[0].direct ? "Отдельный хотфикс для версии 1.3.2." : "Установщики применяются по порядку.")
    : state.version === target
      ? "Все переходы из этой цепочки установлены."
      : state.game
        ? "Для расчёта патчей нужна версия из цепочки."
        : "После проверки появятся нужные переходы.";
  badge($("patch-count"), steps.length ? String(steps.length) : "");
  sync();
}

async function loadPlan(version, keep = false) {
  state.version = version;
  const steps = await invoke("plan", { version });
  renderPlan(steps, keep);
  if (version === target && $("result").hidden) result("Игра уже на версии 1.3.3.");
}

async function verifyGame() {
  resetGame();
  badge($("game-badge"), "Проверка", "busy");
  status("Читаем версию nightreign.exe", "busy");
  const game = await invoke("verify", { path: $("game-path").value });
  state.game = game;
  log(game.gameDir);
  log(game.rawVersion ? "Версия файла " + game.rawVersion : "Метаданные версии отсутствуют.");
  badge($("game-badge"), "Папка найдена", "ok");
  await refreshMod();
  if (game.supported) {
    await loadPlan(game.version);
    status("Найдена версия " + game.version, "ok");
  } else if (!game.version) {
    $("manual").hidden = false;
    gameMessage("Версию файла прочитать не удалось.");
    status("Выберите установленную версию");
  } else {
    state.version = game.version;
    renderPlan([]);
    gameMessage("Версия " + game.version + " вне цепочки 1.3.0–1.3.3.");
    status("Нет подходящего перехода");
  }
}

function makeRow(step, options) {
  const el = $("patch-template").content.firstElementChild.cloneNode(true);
  const row = { step, el, working: false, done: false, error: "", ...options };
  const select = name => el.querySelector("." + name);
  Object.assign(row, {
    badge: select("patch-state"),
    path: select("local-path"),
    packagePath: select("package-path"),
    bar: select("progress"),
    progress: el.querySelector("progress"),
    progressText: select("progress-text"),
  });
  select("patch-title").textContent = step.from + " → " + step.to;
  select("patch-kind").textContent = (step.direct ? "Хотфикс · замена файлов" : "Установщик") + " · " + step.size;
  select("source-hint").textContent = step.direct ? "Папка с файлами хотфикса или архив ZIP/RAR." : "Папка с установщиком, файл EXE или архив ZIP/RAR.";
  row.path.value = row.localText;
  const mirrors = select("mirror");
  for (const mirror of step.mirrors) mirrors.add(new Option(mirrorLabel[mirror], mirror));
  if (row.mirror) mirrors.value = row.mirror;
  select("no-mirrors").hidden = !!row.mirror;
  select("mirror-row").hidden = !row.mirror;
  select("mode-local").onclick = () => setMode(row, "local");
  select("mode-download").onclick = () => setMode(row, "download");
  mirrors.onchange = () => {
    row.mirror = mirrors.value;
    row.download = null;
    resetRowProgress(row);
    sync();
  };
  row.path.oninput = () => {
    row.localText = row.path.value;
    row.local = null;
    resetRowProgress(row);
    sync();
  };
  row.path.onkeydown = event => {
    if (event.key === "Enter" && row.path.value.trim()) run(() => locate(row), message => markError(row, message));
  };
  select("local-find").onclick = () => run(() => locate(row), message => markError(row, message));
  select("local-browse").onclick = () => run(async () => {
    const path = await invoke("browse");
    if (path) {
      row.path.value = path;
      row.localText = path;
      await locate(row);
    }
  }, message => markError(row, message));
  select("mirror-fetch").onclick = () => run(() => prepare(row), message => markError(row, message));
  select("config-folder").onclick = () => openFolder("config");
  select("refresh").onclick = () => run(async () => {
    await loadPlan(state.version, true);
    status("Список зеркал обновлён", "ok");
  });
  return row;
}

function resetRowProgress(row) {
  row.error = "";
  row.bar.hidden = true;
}

function setMode(row, mode) {
  if (row.mode === mode) return;
  row.mode = mode;
  resetRowProgress(row);
  sync();
}

function renderRow(row) {
  const select = name => row.el.querySelector("." + name);
  const canEdit = !state.busy && !row.done;
  row.el.querySelectorAll("button, input, select").forEach(control => { control.disabled = !canEdit; });
  select("local-find").disabled = !canEdit || !row.path.value.trim();
  select("mirror-fetch").disabled = !canEdit || !!row.download;
  select("mode-local").setAttribute("aria-pressed", String(row.mode === "local"));
  select("mode-download").setAttribute("aria-pressed", String(row.mode === "download"));
  select("local-source").hidden = row.mode !== "local" || row.done;
  select("mirror-source").hidden = row.mode !== "download" || row.done;
  select("source-tabs").hidden = row.done;
  const packageFile = readyPackage(row);
  const message = row.error || packageFile?.path || "";
  row.packagePath.hidden = !message || row.done;
  row.packagePath.textContent = message;
  row.packagePath.dataset.tone = row.error ? "error" : "ok";
  const text = row.done ? "Установлен" : row.working ? "В работе" : row.error ? "Ошибка" : packageFile ? "Готов" : "Ожидает пакет";
  const tone = row.done ? "ok" : row.working ? "busy" : row.error ? "error" : packageFile ? "ok" : "";
  badge(row.badge, text, tone);
}

function markError(row, message) {
  row.error = message;
  row.bar.hidden = true;
}

async function locate(row) {
  resetRowProgress(row);
  row.local = null;
  const packageFile = await invoke("locate", { from: row.step.from, path: row.path.value });
  row.local = packageFile;
  log("Выбран пакет " + packageFile.path);
  status("Пакет готов к установке", "ok");
}

function showProgress(row, text, fraction = null) {
  row.bar.hidden = false;
  row.progressText.textContent = text;
  if (fraction === null) row.progress.removeAttribute("value");
  else row.progress.value = Math.min(1, Math.max(0, fraction));
}

async function prepare(row) {
  resetRowProgress(row);
  row.working = true;
  renderRow(row);
  showProgress(row, "Подключение");
  status("Загрузка патча " + row.step.to, "busy");
  try {
    row.download = await invoke("download_patch", { from: row.step.from, mirror: row.mirror });
    showProgress(row, "Готово", 1);
    status("Патч " + row.step.to + " загружен", "ok");
  } finally {
    row.working = false;
    renderRow(row);
  }
}

async function installAll() {
  const rows = remainingRows();
  if (!rows.length || !rows.every(readyPackage)) return;
  result("Идёт установка патчей.", "busy");
  let confirmed = true;
  try {
    for (const row of rows) {
      resetRowProgress(row);
      row.working = true;
      renderRow(row);
      showProgress(row, row.step.direct ? "Подготовка файлов" : "Запуск установщика");
      status("Установка патча " + row.step.to, "busy");
      try {
        const outcome = await invoke("install_patch", {
          from: row.step.from, root: state.game.root, path: readyPackage(row).path,
        });
        row.done = true;
        confirmed &&= outcome.confirmed;
        state.version = outcome.version;
        showProgress(row, "Готово", 1);
        renderChain();
      } catch (error) {
        markError(row, typeof error === "string" ? error : error.message ?? String(error));
        throw error;
      } finally {
        row.working = false;
        renderRow(row);
      }
    }
  } catch (error) {
    result();
    throw error;
  } finally {
    await refreshMod();
  }
  const version = state.version;
  const message = confirmed
    ? (version === "1.3.2" ? "Установлена версия 1.3.2. Хотфикс доступен отдельным шагом." : "Установлена версия " + version + ".")
    : "Патчи применены. Версию " + version + " не удалось подтвердить по файлу.";
  result(message);
  status(message, "ok");
  await loadPlan(version);
}

function bytes(value) {
  if (value >= 1024 ** 3) return (value / 1024 ** 3).toFixed(2) + " ГиБ";
  return (value / 1024 ** 2).toFixed(1) + " МиБ";
}

function renderMod() {
  const view = state.mod;
  const offer = view?.latest;
  const labels = {
    absent: "Не установлен", manual: "Ручная установка",
    changed: (view?.installed ?? "—") + " · файлы изменились",
    installed: view?.installed,
  };
  $("mod-installed").textContent = labels[view?.state] ?? "Папка не выбрана";
  $("mod-latest").textContent = offer ? offer.version + " · " + bytes(offer.size) : "—";
  $("mod-check").disabled = state.busy;
  $("mod-download").disabled = state.busy || !offer || offer.downloaded || !view.needsInstall;
  $("mod-install").disabled = state.busy || !state.game || !offer?.downloaded || !view.needsInstall;
  const label = view?.newerInstalled ? "Установлена более новая версия"
    : offer && !view.needsInstall ? "Актуальная версия"
    : offer?.downloaded ? "Архив скачан"
    : offer && view?.installed && view.installed !== offer.version ? "Есть обновление" : "";
  badge($("mod-badge"), label, label ? "ok" : "");
  const mainAction = offer && view.needsInstall
    ? (offer.downloaded && state.game ? "mod-install" : !offer.downloaded ? "mod-download" : "mod-check")
    : "mod-check";
  for (const id of ["mod-check", "mod-download", "mod-install"]) {
    $(id).classList.toggle("primary", id === mainAction);
    $(id).classList.toggle("secondary", id !== mainAction);
  }
}

function modNote(message = "", tone = "") {
  const note = $("mod-note");
  note.textContent = message;
  note.dataset.tone = tone;
  note.hidden = !message;
}

const modRoot = () => state.game?.root ?? null;

async function refreshMod() {
  try {
    state.mod = await invoke("mod_status", { root: modRoot() });
  } catch (error) {
    const message = String(error);
    log(message, "error");
    modNote(message, "error");
  }
  renderMod();
}

function modProgress(text, fraction = null) {
  $("mod-progress").hidden = false;
  $("mod-progress-text").textContent = text;
  const bar = $("mod-progress-bar");
  if (fraction === null) bar.removeAttribute("value");
  else bar.value = Math.min(1, Math.max(0, fraction));
}

function onModProgress({ payload: event }) {
  if (event.message) log(event.message);
  if (event.phase === "download") {
    modProgress(event.total ? bytes(event.done) + " / " + bytes(event.total) : "Скачивание",
      event.total ? event.done / event.total : null);
  } else {
    modProgress("Файлы " + event.done + " / " + event.total, event.done / event.total);
  }
}

function askModCompatibility(version) {
  const dialog = $("mod-compat");
  $("mod-compat-text").textContent = "regulation.bin отличается от проверенного для 1.3.3."
    + (version ? " Версия игры " + version + "." : "")
    + " Механики мода могут работать некорректно.";
  $("mod-compat-path").textContent = state.game.gameDir;
  dialog.returnValue = "cancel";
  dialog.showModal();
  return new Promise(resolve => {
    dialog.addEventListener("close", () => resolve(dialog.returnValue === "install"), { once: true });
  });
}

const modError = message => {
  $("mod-progress").hidden = true;
  modNote(message, "error");
};

$("mod-check").onclick = () => run(async () => {
  $("mod-progress").hidden = true;
  status("Проверка релизов SecularNightreign", "busy");
  modNote("Проверяем GitHub Releases…");
  state.mod = await invoke("mod_check", { root: modRoot() });
  const offer = state.mod.latest;
  const message = offer ? "Найден релиз " + offer.version : "В релизах пока нет SecularNightreign.zip";
  status(message, offer ? "ok" : "");
  log(message);
  modNote(offer ? "" : message);
}, modError);

$("mod-download").onclick = () => run(async () => {
  status("Скачивание SecularNightreign.zip", "busy");
  modNote();
  modProgress("Подключение");
  state.mod = await invoke("mod_download", { id: state.mod.latest.id, root: modRoot() });
  modProgress("Архив скачан", 1);
  const message = state.game ? "Архив готов к установке" : "Архив скачан. Проверьте папку игры.";
  status(message, "ok");
  log("SecularNightreign.zip скачан");
  modNote(state.game ? "" : message);
}, modError);

$("mod-install").onclick = () => run(async () => {
  const params = { id: state.mod.latest.id, root: modRoot(), allowDifferentRegulation: false };
  modNote();
  modProgress("Подготовка файлов");
  status("Установка SecularNightreign", "busy");
  let outcome = await invoke("mod_install", params);
  if (outcome.kind === "confirm") {
    $("mod-progress").hidden = true;
    if (!await askModCompatibility(outcome.gameVersion)) {
      status("Установка мода отменена");
      return;
    }
    modProgress("Подготовка файлов");
    outcome = await invoke("mod_install", { ...params, allowDifferentRegulation: true });
  }
  state.mod = outcome.view;
  const message = "Установлен SecularNightreign " + state.mod.installed;
  modProgress("Установлен", 1);
  status(message, "ok");
  log(message);
}, modError);
$("mod-cancel").onclick = () => $("mod-compat").close("cancel");
$("mod-confirm").onclick = () => $("mod-compat").close("install");

function onProgress({ payload: event }) {
  const row = state.rows.find(row => row.step.from === event.from);
  if (!row?.working) return;
  if (event.message) log(event.message);
  const detail = [];
  if (event.bytes !== null) detail.push(bytes(event.bytes) + (event.total ? " / " + bytes(event.total) : ""));
  if (event.bytesPerSecond > 0) detail.push(bytes(event.bytesPerSecond) + "/с");
  if (event.elapsed !== null) detail.push(event.elapsed + " с");
  showProgress(row, [phaseLabel[event.phase], ...detail].join(" · "), event.fraction);
}

const openFolder = kind => run(() => invoke("open_folder", { kind, root: state.game?.root ?? null }));

$("game-path").oninput = () => {
  resetGame();
  status("Ожидание проверки игры");
};
$("game-browse").onclick = () => run(async () => {
  const path = await invoke("browse");
  if (path) {
    $("game-path").value = path;
    resetGame();
    status("Папка выбрана");
  }
});
$("game-form").onsubmit = event => {
  event.preventDefault();
  run(verifyGame, message => {
    badge($("game-badge"), "Ошибка", "error");
    gameMessage(message, "error");
  });
};
$("manual-version").onchange = () => run(async () => {
  result();
  const version = $("manual-version").value;
  if (version) {
    await loadPlan(version);
    status("Выбрана версия " + version, "ok");
  } else {
    state.version = null;
    renderPlan([]);
    status("Выберите установленную версию");
  }
});
$("download-all").onclick = () => run(async () => {
  for (const row of pending()) {
    try {
      await prepare(row);
    } catch (error) {
      markError(row, typeof error === "string" ? error : error.message ?? String(error));
      throw error;
    }
  }
});
$("install").onclick = () => run(installAll);
$("open-downloads").onclick = () => openFolder("downloads");
$("open-game").onclick = () => openFolder("game");
$("journal-open").onclick = () => $("journal").showModal();
$("journal-close").onclick = () => $("journal").close();

if (tauri) {
  const windowHandle = tauri.window.getCurrentWindow();
  $("close").onclick = () => windowHandle.close();
  $("maximize").onclick = () => windowHandle.toggleMaximize();
  $("minimize").onclick = () => windowHandle.minimize();
  await tauri.event.listen("patch-progress", onProgress);
  await tauri.event.listen("mod-progress", onModProgress);
  await tauri.event.listen("updater-busy", () => status("Дождитесь завершения операции", "busy"));
}
