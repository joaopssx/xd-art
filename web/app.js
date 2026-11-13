// The page around the wasm engine: read a file, build a recipe from the controls,
// convert, show and offer the result for download. Everything stays in the browser.
import init, { convert, names } from "./pkg/xd_art_wasm.js";

const $ = (id) => document.getElementById(id);
const status = (text) => { $("status").textContent = text; };

let source = null; // the chosen file's bytes
let resultUrl = null;
let timer = 0;

function fill(select, items, chosen) {
  for (const name of items) {
    const o = document.createElement("option");
    o.value = o.textContent = name;
    select.append(o);
  }
  select.value = chosen;
}

// The recipe format is the one the CLI and the desktop app read; see docs/recipes.md.
function recipe() {
  const keep = $("keep").checked;
  const sample = { mode: $("sample").value, width: Number($("width").value) };
  if (keep) sample.keep_aspect = true;
  else sample.height = Number($("height").value);
  return JSON.stringify({
    "xd-art": 1,
    sample,
    colors: { palette: $("palette").value, metric: $("metric").value },
    dither: { mode: $("dither").value, amount: Number($("amount").value) },
    output: { scale: Number($("scale").value) },
  });
}

function run() {
  if (!source) return;
  const started = performance.now();
  try {
    const png = convert(source, recipe());
    if (resultUrl) URL.revokeObjectURL(resultUrl);
    resultUrl = URL.createObjectURL(new Blob([png], { type: "image/png" }));
    $("result").src = resultUrl;
    $("result").hidden = false;
    $("download").href = resultUrl;
    $("download").setAttribute("aria-disabled", "false");
    status(`converted in ${Math.round(performance.now() - started)} ms`);
  } catch (e) {
    status(e.message || String(e));
  }
}

// Sliders and number fields fire many events; convert once they stop for a moment.
function later() {
  clearTimeout(timer);
  timer = setTimeout(run, 60);
}

async function main() {
  try {
    await init();
  } catch (e) {
    status(`the engine did not load: ${e.message || e}`);
    return;
  }
  fill($("sample"), names("samplers"), "nearest");
  fill($("palette"), names("palettes"), "pico-8");
  fill($("metric"), names("metrics"), "redmean");
  fill($("dither"), names("dithers"), "bayer4x4");
  status("choose an image");

  $("file").addEventListener("change", async () => {
    const file = $("file").files[0];
    if (!file) return;
    source = new Uint8Array(await file.arrayBuffer());
    if ($("source").src) URL.revokeObjectURL($("source").src);
    $("source").src = URL.createObjectURL(file);
    $("source").hidden = false;
    $("download").download = file.name.replace(/\.[^.]*$/, "") + "-px.png";
    run();
  });
  $("keep").addEventListener("change", () => {
    $("height").disabled = $("keep").checked;
    later();
  });
  $("height").disabled = $("keep").checked;
  $("amount").addEventListener("input", () => {
    $("amount-value").textContent = $("amount").value;
  });
  for (const id of ["width", "height", "sample", "palette", "metric", "dither", "amount", "scale"]) {
    $(id).addEventListener("input", later);
  }
}

main();
