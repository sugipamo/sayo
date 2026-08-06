import { renderTermSource, termKind } from "./term-view.js?v=20260806.47";
import { animateRewrite, resetMotion, snapshotFragments } from "./trace-motion.js?v=20260806.50";

const $ = (selector) => document.querySelector(selector);
const ASSET_VERSION = "20260806.48";
const STAGE_TIMING = { leaving: 520 };
const PLAYBACK_SETTLE_MARGIN = 30;
const AUTOPLAY_DELAY = 650;
const FOLLOW_EDGE_INSET = 0.2;
const NESTED_ROW_SELECTOR = ":scope > .nested-context-row, :scope > .nested-child-row";
const ACTIVE_ROW_SELECTOR = ".nested-context-row, .nested-child-row";

const samples = {
  showcase: {
    label: "Showcase · nested motion",
    entry: "main.sayo",
    files: { "main.sayo": "for (add (fold add 0 1 2 3)) 4 5 6 7 4 5 6 7 4 5 6 7 4 5 6 7" },
    directories: [],
  },
  arithmetic: {
    label: "Arithmetic · native",
    entry: "main.sayo",
    files: { "main.sayo": "add 5 3" },
    directories: [],
  },
  fold: {
    label: "Fold · standard world",
    entry: "main.sayo",
    files: { "main.sayo": "fold add 0 1 2 3" },
    directories: [],
  },
  duplicate: {
    label: "Duplicate · standard world",
    entry: "main.sayo",
    files: { "main.sayo": "duplicate 5" },
    directories: [],
  },
  motion: {
    label: "For · leftward emit",
    entry: "main.sayo",
    files: { "main.sayo": "for (add 9) 1 2 3" },
    directories: [],
  },
  structural: {
    label: "Structural capture · user world",
    entry: "main.sayo",
    files: {
      "main.sayo": "box 5",
      "Box/space.sayo": 'equal-to "box" %0',
    },
    directories: ["Box/Int/Boxed", "Boxed"],
  },
  trap: {
    label: "Trap · division by zero",
    entry: "main.sayo",
    files: { "main.sayo": "div 12 0" },
    directories: [],
  },
};

let project = structuredClone(samples.showcase);
let worker = null;
let runId = 0;
let running = false;
let visualization = null;
let frames = [];
let frameIndex = 0;
let playTimer = null;
let autoplayTimer = null;
let toastTimer = null;
let projectDirty = true;
let sourceUpdateTimer = null;
let currentFailure = null;

const editor = $("#source-editor");
const lineNumbers = $("#line-numbers");
const actionButton = $("#play-button");
const statusPill = $("#status-pill");
const timeline = $("#timeline");
const eventList = $("#event-list");

function initialize() {
  const select = $("#sample-select");
  for (const [value, sample] of Object.entries(samples)) {
    select.add(new Option(sample.label, value));
  }
  select.value = "showcase";
  bindEvents();
  loadHash().then((restoredSharedProject) => {
    syncEditor();
    updateWorldSummary();
    execute({
      playWhenReady: !restoredSharedProject && initialAutoplayEnabled(),
      playbackSpeed: "150",
    });
  });
}

function bindEvents() {
  editor.addEventListener("input", () => {
    cancelScheduledPlayback();
    project.files[project.entry] = editor.value;
    projectDirty = true;
    setStatus("ready", "Edited");
    updateActionButton();
    updateLineNumbers();
    $("#sample-select").value = "";
    clearTimeout(sourceUpdateTimer);
    sourceUpdateTimer = setTimeout(() => execute({ background: true }), 500);
  });
  editor.addEventListener("scroll", () => { lineNumbers.scrollTop = editor.scrollTop; });
  $("#sample-select").addEventListener("change", (event) => {
    if (!event.target.value) return;
    loadSample(event.target.value);
  });
  actionButton.addEventListener("click", () => {
    cancelScheduledPlayback();
    if (running) stopRun();
    else if (projectDirty || !visualization) execute();
    else togglePlayback();
  });
  $("#world-button").addEventListener("click", openWorldEditor);
  $("#add-file").addEventListener("click", () => addFileCard("new.sayo", ""));
  $("#save-world").addEventListener("click", saveWorldEditor);
  $("#share-button").addEventListener("click", shareProject);
  $("#export-button").addEventListener("click", exportProject);
  $("#import-button").addEventListener("click", () => $("#file-input").click());
  $("#file-input").addEventListener("change", importProject);
  $("#step-back").addEventListener("click", () => { cancelScheduledPlayback(); setFrame(frameIndex - 1); });
  $("#step-forward").addEventListener("click", () => { cancelScheduledPlayback(); setFrame(frameIndex + 1); });
  $("#jump-start").addEventListener("click", () => { cancelScheduledPlayback(); stopPlayback(); setFrame(0); });
  $("#jump-end").addEventListener("click", () => { cancelScheduledPlayback(); stopPlayback(); setFrame(frames.length - 1); });
  timeline.addEventListener("input", () => { cancelScheduledPlayback(); setFrame(Number(timeline.value)); });
  document.querySelectorAll(".example-card").forEach((card) => {
    card.addEventListener("click", () => {
      loadSample(card.dataset.sample, { playWhenReady: true });
      $("#editor-title").scrollIntoView({ behavior: "smooth", block: "start" });
    });
  });
  window.addEventListener("keydown", (event) => {
    if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
      event.preventDefault();
      running ? stopRun() : execute();
    } else if (!event.metaKey && !event.ctrlKey && event.key.toLowerCase() === "w" && document.activeElement === document.body) {
      event.preventDefault();
      openWorldEditor();
    }
  });
  window.addEventListener("hashchange", () => loadHash());
}

function loadSample(name, { playWhenReady = false } = {}) {
  const sample = samples[name];
  if (!sample) return;
  cancelScheduledPlayback();
  project = structuredClone(sample);
  $("#sample-select").value = name;
  syncEditor();
  updateWorldSummary();
  resetEvaluation();
  execute({ playWhenReady });
}

function initialAutoplayEnabled() {
  return new URLSearchParams(location.search).get("autoplay") !== "0"
    && !window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

function syncEditor() {
  $("#editor-title").textContent = project.entry;
  editor.value = project.files[project.entry] ?? "";
  updateLineNumbers();
}

function updateLineNumbers() {
  const count = Math.max(1, editor.value.split("\n").length);
  lineNumbers.textContent = Array.from({ length: count }, (_, index) => index + 1).join("\n");
}

function updateWorldSummary() {
  const files = Object.keys(project.files).length;
  const dirs = project.directories.length;
  $("#world-summary").textContent = `${files} file${files === 1 ? "" : "s"}${dirs ? ` · ${dirs} empty dir${dirs === 1 ? "" : "s"}` : ""} · standard world included`;
}

function createWorker() {
  worker?.terminate();
  worker = new Worker(`./sayo-worker.js?v=${ASSET_VERSION}`, { type: "module" });
  return worker;
}

async function execute({ background = false, playWhenReady = false, playbackSpeed = null } = {}) {
  clearTimeout(sourceUpdateTimer);
  sourceUpdateTimer = null;
  stopPlayback();
  project.files[project.entry] = editor.value;
  running = true;
  runId += 1;
  const id = runId;
  const executedProject = JSON.stringify(projectDocument());
  setRunningUi(true);
  setStatus("running", "Running");

  const activeWorker = createWorker();
  const request = {
    version: 1,
    entry: project.entry,
    files: project.files,
    directories: project.directories,
    options: {
      visualization: true,
      relation_depth: 256,
      max_steps: boundedNumber($("#step-limit").value, 100000),
      max_trace_events: boundedNumber($("#trace-limit").value, 100000),
    },
  };

  activeWorker.onmessage = (event) => {
    if (event.data?.id !== id) return;
    const response = JSON.parse(event.data.response);
    running = false;
    projectDirty = executedProject !== JSON.stringify(projectDocument());
    setRunningUi(false);
    if (background && response.status !== "normal" && !response.visualization) {
      projectDirty = true;
      setStatus("ready", "Edited");
      updateActionButton();
      return;
    }
    renderResponse(response);
    if (playWhenReady && response.status === "normal") schedulePlayback(id, playbackSpeed);
  };
  activeWorker.onerror = (event) => {
    if (id !== runId) return;
    running = false;
    setRunningUi(false);
    renderResponse({ status: "trap", trap: { category: "host error", message: event.message } });
  };
  activeWorker.postMessage({ id, request });
}

function schedulePlayback(id, playbackSpeed) {
  cancelScheduledPlayback();
  autoplayTimer = setTimeout(() => {
    autoplayTimer = null;
    if (id !== runId || running || frames.length <= 1 || frameIndex !== 0) return;
    if (playbackSpeed) $("#speed-select").value = playbackSpeed;
    togglePlayback();
  }, AUTOPLAY_DELAY);
}

function cancelScheduledPlayback() {
  clearTimeout(autoplayTimer);
  autoplayTimer = null;
}

function stopRun() {
  if (!running) return;
  worker?.terminate();
  worker = null;
  runId += 1;
  running = false;
  setRunningUi(false);
  setStatus("ready", "Stopped");
  showToast("Execution stopped");
}

function boundedNumber(value, fallback) {
  const number = Number(value);
  return Number.isSafeInteger(number) && number > 0 ? Math.min(number, 1_000_000) : fallback;
}

function setRunningUi(value) {
  actionButton.classList.toggle("running", value);
  updateActionButton();
}

function updateActionButton() {
  if (running) {
    actionButton.textContent = "■";
    actionButton.setAttribute("aria-label", "Stop Sayo program");
  } else if (playTimer) {
    actionButton.textContent = "Ⅱ";
    actionButton.setAttribute("aria-label", "Pause reduction playback");
  } else {
    actionButton.textContent = "▶";
    actionButton.setAttribute("aria-label", projectDirty || !visualization ? "Run Sayo program" : "Play reduction");
  }
}

function renderResponse(response) {
  visualization = response.visualization ?? null;
  if (response.status === "normal") {
    currentFailure = null;
    setStatus("normal", "Normal");
  } else {
    const failure = response.trap ?? response.limit ?? { message: "Evaluation did not complete." };
    currentFailure = {
      title: response.status === "limit" ? "Bootstrap limit" : failure.category || "Trap",
      message: failure.message,
    };
    setStatus(response.status === "limit" ? "limit" : "trap", response.status);
    if (!visualization?.frames?.some((frame) => frame.cause.kind === "trapped")) {
      showToast(`${currentFailure.title}: ${currentFailure.message}`);
    }
  }
  renderVisualization(visualization);
}

function setStatus(kind, label) {
  statusPill.className = `status-pill ${kind}`;
  statusPill.textContent = label;
}

function resetEvaluation() {
  clearTimeout(sourceUpdateTimer);
  sourceUpdateTimer = null;
  stopPlayback();
  cancelScheduledPlayback();
  if (worker) {
    worker.terminate();
    worker = null;
  }
  if (running) runId += 1;
  running = false;
  setRunningUi(false);
  visualization = null;
  currentFailure = null;
  frames = [];
  frameIndex = 0;
  projectDirty = true;
  setStatus("ready", "Ready");
  renderVisualization(null);
  updateActionButton();
}

function renderVisualization(traceDocument) {
  stopPlayback();
  eventList.replaceChildren();
  if (!traceDocument?.frames?.length) {
    const stage = $("#term-stage");
    resetMotion(stage);
    stage.replaceChildren();
    stage.classList.remove("nested", "instant");
    stage.removeAttribute("style");
    frames = [];
    frameIndex = 0;
    timeline.max = 0;
    timeline.value = 0;
    $("#step-count").textContent = "0 / 0";
    $("#event-total").textContent = "0 events";
    $("#event-detail").textContent = "No trace loaded";
    $("#stage-placeholder").hidden = false;
    stage.hidden = true;
    return;
  }
  frames = traceDocument.frames;
  frames.forEach((frame, index) => {
    const item = document.createElement("li");
    item.innerHTML = `<span class="event-index">#${frame.frame_id}</span><span class="event-kind">${escapeHtml(frame.cause.kind)}</span><span>${escapeHtml(frame.cause.summary)}</span>`;
    item.addEventListener("click", () => setFrame(index));
    eventList.append(item);
  });
  timeline.max = frames.length - 1;
  $("#event-total").textContent = `${frames.length} frames`;
  setFrame(0);
}

function setFrame(nextIndex) {
  if (!frames.length) return;
  const previousIndex = frameIndex;
  frameIndex = Math.max(0, Math.min(nextIndex, frames.length - 1));
  const animate = frameIndex > previousIndex;
  const frame = frames[frameIndex];
  timeline.value = frameIndex;
  $("#step-count").textContent = `${frameIndex + 1} / ${frames.length}`;
  $("#event-detail").textContent = `#${frame.frame_id} · ${frame.cause.kind} · ${frame.cause.summary}`;
  if (frame.cause.kind === "trapped") {
    const failure = currentFailure ?? { title: "Trap", message: frame.cause.summary };
    showToast(`${failure.title}: ${failure.message}`);
  }
  [...eventList.children].forEach((item, index) => item.classList.toggle("current", index === frameIndex));
  eventList.children[frameIndex]?.scrollIntoView({ block: "nearest" });
  const motionBudget = playTimer
    ? Math.max(24, Number($("#speed-select").value) - PLAYBACK_SETTLE_MARGIN)
    : Infinity;
  renderStage(frame, { animate, motionBudget });
}

function renderStage(frame, { animate, motionBudget }) {
  const stage = $("#term-stage");
  const stageViewport = $("#stage");
  prepareStage(stage, animate);
  configureMotionBudget(stage, motionBudget);
  const focusedViews = frame.focus?.view_ids ?? [];
  const trackingViews = focusedViews.length
    ? focusedViews
    : changesTargetViews(frame.changes ?? []);
  const entries = stageEntries(frame);
  const nextKeys = new Set(entries.map((entry) => entry.key));
  const changes = frame.changes ?? [];

  rekeyPreservedCards(stage, changes, nextKeys);
  const oldCards = currentCards(stage);
  const oldRects = new Map([...oldCards].map(([key, card]) => [key, card.getBoundingClientRect()]));
  const oldFragments = snapshotFragments(stage);
  const nested = frame.scene.rows.some((row) => row.role === "context");
  stage.classList.toggle("nested", nested);
  const sceneRows = prepareSceneRows(stage, frame.scene.rows, nested);

  retireCards(stage, oldCards, nextKeys, nestedHandoffSources(changes, nextKeys), animate);
  placeEntries(stage, sceneRows, entries, oldCards, frame, focusedViews, nested, animate);
  removeEmptyNestedRows(stage);
  alignSceneRows(stage);
  animateLayout(stage, oldRects, animate);

  // Start rewrite motion in the same render turn. Waiting for two paints left
  // retired work-row cards visible after their row had already disappeared,
  // which looked like a one-frame duplicate in the top row during playback.
  if (animate) animateRewrite(frame, stage, oldFragments, { maxDuration: motionBudget });
  else requestAnimationFrame(() => stage.classList.remove("instant"));
  nextPaint(() => followActiveTerms(stageViewport, stage, trackingViews));
}

function changesTargetViews(changes) {
  return [...new Set(changes
    .filter((change) => change.kind !== "preserve")
    .flatMap((change) => change.to))];
}

function configureMotionBudget(stage, motionBudget) {
  if (!Number.isFinite(motionBudget)) {
    stage.style.removeProperty("--layout-motion-duration");
    stage.style.removeProperty("--fade-motion-duration");
    return;
  }
  stage.style.setProperty("--layout-motion-duration", `${Math.min(340, motionBudget)}ms`);
  stage.style.setProperty("--fade-motion-duration", `${Math.min(280, motionBudget)}ms`);
}

function prepareStage(stage, animate) {
  resetMotion(stage);
  stage.querySelectorAll(".term-card.leaving").forEach((card) => card.remove());
  stage.classList.toggle("instant", !animate);
  $("#stage-placeholder").hidden = true;
  stage.hidden = false;
}

function currentCards(stage) {
  return new Map([...stage.querySelectorAll(".term-card:not(.leaving)")]
    .map((card) => [card.dataset.key, card]));
}

function prepareSceneRows(stage, rows, nested) {
  if (!nested) return new Map();
  return new Map(rows.map((row) => [row.row_id, nestedRow(
    stage,
    row.role === "context" ? "nested-context-row" : "nested-child-row",
    row.row_id,
    row.separator,
    row.anchor_view_id,
  )]));
}

function retireCards(stage, oldCards, nextKeys, handoffSources, animate) {
  const parentRect = stage.getBoundingClientRect();
  for (const [key, card] of oldCards) {
    if (nextKeys.has(key)) continue;
    if (!animate) {
      card.remove();
      continue;
    }
    const rect = card.getBoundingClientRect();
    stage.append(card);
    Object.assign(card.style, {
      left: `${rect.left - parentRect.left}px`,
      top: `${rect.top - parentRect.top}px`,
      width: `${rect.width}px`,
      height: `${rect.height}px`,
    });
    card.classList.add("leaving");
    card.classList.toggle("handoff-source", handoffSources.has(key));
    setTimeout(() => card.remove(), STAGE_TIMING.leaving);
  }
}

function placeEntries(stage, sceneRows, entries, oldCards, frame, focusedViews, nested, animate) {
  for (const entry of entries) {
    let card = oldCards.get(entry.key);
    const isNew = !card;
    if (!card) {
      card = document.createElement("article");
      card.className = animate ? "term-card entering" : "term-card";
      card.dataset.key = entry.key;
    }
    updateTermCard(card, entry, frame, focusedViews);
    (nested ? sceneRows.get(entry.rowId) : stage).append(card);
    if (animate && isNew) nextPaint(() => card.classList.remove("entering"));
  }
}

function animateLayout(stage, oldRects, animate) {
  if (!animate) return;
  for (const [key, previous] of oldRects) {
    const card = stage.querySelector(`.term-card[data-key="${CSS.escape(key)}"]:not(.leaving)`);
    if (!card) continue;
    const current = card.getBoundingClientRect();
    const deltaX = Math.round(previous.left - current.left);
    const deltaY = Math.round(previous.top - current.top);
    if (deltaX === 0 && deltaY === 0) continue;
    card.style.transition = "none";
    card.style.transform = `translate(${deltaX}px, ${deltaY}px)`;
    nextPaint(() => {
      card.style.transition = "";
      card.style.transform = "";
    });
  }
}

function nextPaint(callback) {
  requestAnimationFrame(() => requestAnimationFrame(callback));
}

function nestedHandoffSources(changes, nextKeys) {
  return new Set(changes
    .filter((change) => change.kind === "preserve"
      && change.from.length === 1
      && change.to.length === 1
      && !nextKeys.has(change.to[0]))
    .map((change) => change.from[0]));
}

function stageEntries(frame) {
  const nested = frame.scene.rows.some((row) => row.role === "context");
  const entries = frame.scene.rows.flatMap((row) => row.items.map((item) => ({
    key: item.view_id,
    viewId: item.view_id,
    cell: item.cell_id ?? null,
    term: item.term,
    context: row.role === "context",
    template: row.role === "template",
    nestedChild: nested && row.role !== "context",
    rowId: row.row_id,
  })));
  return entries.length ? entries : [{ key: "empty", cell: null, term: null, empty: true }];
}

function rekeyPreservedCards(stage, changes, nextKeys) {
  for (const change of changes) {
    if (change.kind !== "preserve" || change.from.length !== 1 || change.to.length !== 1) continue;
    if (!nextKeys.has(change.to[0])) continue;
    const card = stage.querySelector(`.term-card[data-key="${CSS.escape(change.from[0])}"]:not(.leaving)`);
    if (card) card.dataset.key = change.to[0];
  }
}

function nestedRow(stage, className, rowId, separator, anchorViewId) {
  let row = [...stage.querySelectorAll(NESTED_ROW_SELECTOR)]
    .find((candidate) => candidate.dataset.rowId === rowId);
  if (!row) {
    row = document.createElement("div");
    stage.append(row);
  }
  row.className = className;
  row.dataset.rowId = rowId;
  row.dataset.separator = separator ?? "space";
  if (anchorViewId) row.dataset.anchorViewId = anchorViewId;
  else delete row.dataset.anchorViewId;
  stage.append(row);
  return row;
}

function alignSceneRows(stage) {
  for (const row of stage.querySelectorAll(NESTED_ROW_SELECTOR)) {
    const anchorViewId = row.dataset.anchorViewId;
    const anchor = anchorViewId
      ? stage.querySelector(`.term-card[data-key="${CSS.escape(anchorViewId)}"]:not(.leaving)`)
      : null;
    row.style.marginLeft = anchor ? `${layoutBox(anchor, stage).left}px` : "0px";
  }
}

function removeEmptyNestedRows(stage) {
  stage.querySelectorAll(NESTED_ROW_SELECTOR)
    .forEach((row) => {
      if (!row.children.length) row.remove();
    });
}

function followActiveTerms(viewport, stage, trackingViews) {
  const anchorViews = [...stage.querySelectorAll(NESTED_ROW_SELECTOR)]
    .map((row) => row.dataset.anchorViewId)
    .filter(Boolean);
  const activeViews = trackingViews.length ? trackingViews : anchorViews.slice(-1);
  const cards = activeViews
    .map((viewId) => stage.querySelector(`.term-card[data-key="${CSS.escape(viewId)}"]:not(.leaving)`))
    .filter(Boolean);

  const rows = [...stage.querySelectorAll(NESTED_ROW_SELECTOR)];
  const activeRow = cards.at(-1)?.closest(ACTIVE_ROW_SELECTOR) ?? rows.at(-1) ?? stage;
  const rowBox = layoutBox(activeRow, viewport);
  const verticalTarget = viewport.scrollHeight > viewport.clientHeight
    ? Math.max(0, rowBox.bottom - viewport.clientHeight + 48)
    : 0;

  const horizontalTarget = horizontalFollowTarget(viewport, cards);

  viewport.scrollTo({
    left: clampScroll(horizontalTarget, viewport.scrollWidth, viewport.clientWidth),
    top: clampScroll(verticalTarget, viewport.scrollHeight, viewport.clientHeight),
    behavior: "auto",
  });
}

function horizontalFollowTarget(viewport, cards) {
  if (!cards.length || viewport.scrollWidth <= viewport.clientWidth) return viewport.scrollLeft;
  const boxes = cards.map((card) => layoutBox(card, viewport));
  const targetLeft = Math.min(...boxes.map((box) => box.left));
  const targetRight = Math.max(...boxes.map((box) => box.right));
  const visibleLeft = viewport.scrollLeft;
  const visibleRight = visibleLeft + viewport.clientWidth;
  if (targetLeft < visibleLeft) return targetLeft - viewport.clientWidth * FOLLOW_EDGE_INSET;
  if (targetRight > visibleRight) {
    return targetRight - viewport.clientWidth * (1 - FOLLOW_EDGE_INSET);
  }
  return viewport.scrollLeft;
}

function clampScroll(target, scrollSize, viewportSize) {
  return Math.max(0, Math.min(target, scrollSize - viewportSize));
}

function layoutBox(element, ancestor) {
  let left = 0;
  let top = 0;
  let node = element;
  while (node && node !== ancestor) {
    left += node.offsetLeft;
    top += node.offsetTop;
    node = node.offsetParent;
  }
  if (node !== ancestor) {
    const elementRect = element.getBoundingClientRect();
    const ancestorRect = ancestor.getBoundingClientRect();
    left = elementRect.left - ancestorRect.left + ancestor.scrollLeft;
    top = elementRect.top - ancestorRect.top + ancestor.scrollTop;
  }
  return {
    left,
    right: left + element.offsetWidth,
    top,
    bottom: top + element.offsetHeight,
  };
}

function updateTermCard(card, entry, frame, focusedViews) {
  const term = entry.term;
  card.classList.toggle("trapped", frame.focus?.style === "trap" && focusedViews.includes(entry.viewId));
  card.classList.toggle("nested-context", Boolean(entry.context));
  card.classList.toggle("nested-child", Boolean(entry.nestedChild));
  card.classList.toggle("control-template", Boolean(entry.template));
  card.dataset.kind = termKind(term);
  card.dataset.source = entry.empty ? "∅" : term?.source ?? "";
  if (entry.cell === null || entry.context) delete card.dataset.cell;
  else card.dataset.cell = String(entry.cell);
  const source = renderTermSource(entry.empty
    ? { kind: "atom", term_id: null, source: "∅", space: "" }
    : term);
  source.dataset.viewId = entry.viewId ?? entry.key;
  card.replaceChildren(source);
}

function togglePlayback() {
  if (playTimer) { stopPlayback(); return; }
  if (!frames.length) return;
  if (frameIndex >= frames.length - 1) setFrame(0);
  const advance = () => {
    if (frameIndex >= frames.length - 1) { stopPlayback(); return; }
    setFrame(frameIndex + 1);
    playTimer = setTimeout(advance, Number($("#speed-select").value));
  };
  playTimer = setTimeout(advance, Number($("#speed-select").value));
  updateActionButton();
}

function stopPlayback() {
  clearTimeout(playTimer);
  playTimer = null;
  updateActionButton();
}

function openWorldEditor() {
  const list = $("#file-list");
  list.replaceChildren();
  for (const [path, contents] of Object.entries(project.files)) addFileCard(path, contents);
  $("#directory-editor").value = project.directories.join("\n");
  $("#world-dialog").showModal();
}

function addFileCard(path, contents) {
  const card = document.createElement("div");
  card.className = "file-card";
  card.innerHTML = `<div class="file-card-head"><input class="file-path" aria-label="World file path" value="${escapeAttribute(path)}"><button class="remove-file" type="button" aria-label="Remove ${escapeAttribute(path)}">Remove</button></div><textarea class="file-source" rows="5" aria-label="Contents of ${escapeAttribute(path)}">${escapeHtml(contents)}</textarea>`;
  card.querySelector(".remove-file").addEventListener("click", () => card.remove());
  $("#file-list").append(card);
}

function saveWorldEditor(event) {
  event.preventDefault();
  const files = {};
  for (const card of $("#file-list").children) {
    const path = card.querySelector(".file-path").value.trim();
    if (!path) { showToast("Every file needs a path"); return; }
    if (path in files) { showToast(`Duplicate path: ${path}`); return; }
    files[path] = card.querySelector(".file-source").value;
  }
  if (!(project.entry in files)) {
    showToast(`${project.entry} cannot be removed`);
    return;
  }
  project.files = files;
  project.directories = $("#directory-editor").value.split("\n").map((path) => path.trim()).filter(Boolean);
  syncEditor();
  updateWorldSummary();
  resetEvaluation();
  $("#world-dialog").close();
}

function projectDocument() {
  project.files[project.entry] = editor.value;
  return { format: "sayo-playground", version: 0, entry: project.entry, files: project.files, directories: project.directories };
}

function exportProject() {
  const blob = new Blob([JSON.stringify(projectDocument(), null, 2)], { type: "application/json" });
  const link = document.createElement("a");
  link.href = URL.createObjectURL(blob);
  link.download = "sayo-world.json";
  link.click();
  URL.revokeObjectURL(link.href);
  showToast("World exported");
}

async function importProject(event) {
  const file = event.target.files?.[0];
  event.target.value = "";
  if (!file) return;
  try {
    applyProjectDocument(JSON.parse(await file.text()));
    $("#world-dialog").close();
    showToast("World imported");
  } catch (error) {
    showToast(error instanceof Error ? error.message : "Invalid project file");
  }
}

function applyProjectDocument(document) {
  if (document?.format !== "sayo-playground" || document.version !== 0) throw new Error("Unsupported Sayo Playground document");
  if (typeof document.entry !== "string" || !document.files || typeof document.files[document.entry] !== "string" || !Array.isArray(document.directories)) throw new Error("Project document is incomplete");
  project = { entry: document.entry, files: document.files, directories: document.directories };
  syncEditor();
  updateWorldSummary();
  resetEvaluation();
  $("#sample-select").value = "";
}

async function shareProject() {
  try {
    const encoded = await encodeProject(projectDocument());
    const url = `${location.origin}${location.pathname}#p=${encoded}`;
    if (url.length > 8000) {
      showToast("Too large for a reliable URL — export JSON instead");
      return;
    }
    history.replaceState(null, "", `#p=${encoded}`);
    await navigator.clipboard.writeText(url);
    showToast(`Share URL copied · ${url.length.toLocaleString()} characters`);
  } catch {
    showToast("Could not create a share URL");
  }
}

async function loadHash() {
  if (!location.hash.startsWith("#p=")) return false;
  try {
    applyProjectDocument(await decodeProject(location.hash.slice(3)));
    showToast("Shared world restored");
    return true;
  } catch {
    showToast("This share URL is invalid or unsupported");
    return true;
  }
}

async function encodeProject(document) {
  const input = new TextEncoder().encode(JSON.stringify(document));
  const stream = new Blob([input]).stream().pipeThrough(new CompressionStream("gzip"));
  return bytesToBase64Url(new Uint8Array(await new Response(stream).arrayBuffer()));
}

async function decodeProject(encoded) {
  const bytes = base64UrlToBytes(encoded);
  const stream = new Blob([bytes]).stream().pipeThrough(new DecompressionStream("gzip"));
  return JSON.parse(await new Response(stream).text());
}

function bytesToBase64Url(bytes) {
  let binary = "";
  for (let index = 0; index < bytes.length; index += 0x8000) binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
}

function base64UrlToBytes(value) {
  const normalized = value.replaceAll("-", "+").replaceAll("_", "/").padEnd(Math.ceil(value.length / 4) * 4, "=");
  return Uint8Array.from(atob(normalized), (character) => character.charCodeAt(0));
}

function showToast(message) {
  const toast = $("#toast");
  toast.textContent = message;
  toast.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => toast.classList.remove("show"), 2600);
}

function escapeHtml(value) {
  return String(value).replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;").replaceAll("'", "&#039;");
}

function escapeAttribute(value) { return escapeHtml(value); }

initialize();
