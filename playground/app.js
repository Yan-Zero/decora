// Decora — Live WASM Protocol Converter Demo
// Invokes the compiled Rust `decora_wasm.wasm` module directly via linear-memory JSON ABI.

const DIALECT_ENDPOINTS = {
  openai: "POST /v1/decisions",
  gemini: "POST /v1beta/decisions",
  typesafe: "POST /v0/decisions",
  openrouter: "POST /api/v1/decisions",
};

const OPENAI_SUPPORT_REQUEST = {
  model: "decora-gemini",
  input: [
    { role: "system", content: "You are an after-sales triage assistant." },
    {
      role: "user",
      content: [
        { type: "input_text", text: "Package arrived damaged; please replace it today." },
        { type: "input_json", data: { order: "A17", damaged: true } },
      ],
    },
  ],
  questions: [
    { type: "predicate", name: "damaged", instructions: "Is the package damaged?" },
    {
      type: "choice",
      name: "team",
      instructions: "Select the routing label.",
      choices: [{ value: true }, { value: "true" }, { value: "售后" }],
    },
    {
      type: "score",
      name: "urgency",
      instructions: "Rate handling urgency.",
      levels: [{ label: "Low" }, { label: "High", description: "Needs handling today" }],
    },
  ],
};

const GEMINI_NATIVE_REQUEST = {
  model: "decora-gemini",
  input: "The package arrived damaged; please replace it today.",
  questions: {
    damage: { type: "noul", instructions: "Is the package damaged?" },
    route: {
      type: "choice",
      instructions: "Select handling team",
      criteria: { support: "After-sales support", sales: "Sales team" },
    },
    urgency: {
      type: "score",
      instructions: "Rate urgency",
      criteria: ["Low", "High"],
    },
  },
};

const PRESETS = [
  {
    id: "cross-translate",
    title: "1. Cross-Dialect Translation",
    sub: "OpenAI roles + JSON + mixed true/\"true\"/\"售后\" → Gemini",
    note: "Shows openai_decisions_text_v1 encoding, decora_choice_0..2, x-goog-api-key, and usage.decora.",
    dialect: "openai",
    target: "gemini",
    usagePolicy: "compatible",
    failover: false,
    backup: "typesafe",
    phantom: false,
    request: OPENAI_SUPPORT_REQUEST,
  },
  {
    id: "passthrough",
    title: "2. Same-Dialect Passthrough",
    sub: "Gemini → Gemini preserves native keys & native usage",
    note: "Native keys (damage/route/urgency, support/sales) and native usage pass untouched — zero usage.decora.",
    dialect: "gemini",
    target: "gemini",
    usagePolicy: "strict",
    failover: false,
    backup: "openai",
    phantom: false,
    request: GEMINI_NATIVE_REQUEST,
  },
  {
    id: "failover",
    title: "3. 503 Failover Switch",
    sub: "Gemini passthrough 503 → OpenAI Canonical IR",
    note: "Attempt 1 uses same-dialect passthrough; on 503, Attempt 2 switches per-attempt to Canonical IR.",
    dialect: "gemini",
    target: "gemini",
    usagePolicy: "compatible",
    failover: true,
    backup: "openai",
    phantom: false,
    request: GEMINI_NATIVE_REQUEST,
  },
  {
    id: "phantom",
    title: "4. Phantom Rejection (502)",
    sub: "Upstream Σp = 0.93 blocked — never normalized",
    note: "Simulated upstream returns 0.05 + 0.10 + 0.78 = 0.93. Fix 0.78 → 0.85 and hit Re-Verify to pass.",
    dialect: "openai",
    target: "openai",
    usagePolicy: "compatible",
    failover: false,
    backup: "typesafe",
    phantom: true,
    request: { ...OPENAI_SUPPORT_REQUEST, model: "decora-openai" },
  },
  {
    id: "strict-dialect",
    title: "5. Strict Dialect Guard (400)",
    sub: "Sending TypeSafe \"state\" to OpenAI /v1/decisions",
    note: "Each inbound endpoint enforces its exact official dialect schema with zero cross-contamination.",
    dialect: "openai",
    target: "openai",
    usagePolicy: "compatible",
    failover: false,
    backup: "typesafe",
    phantom: false,
    request: {
      model: "decora-openai",
      state: "Package arrived damaged.",
      questions: { damage: { type: "noul", instructions: "Is it damaged?" } },
    },
  },
  {
    id: "json-vs-image",
    title: "6. JSON Data vs. Strict Usage",
    sub: "input_json.image_url passes text-only TypeSafe",
    note: "image_url inside input_json is plain data (passes). Switch usage_policy to strict to see 400 unsupported_usage!",
    dialect: "openai",
    target: "typesafe",
    usagePolicy: "compatible",
    failover: false,
    backup: "openai",
    phantom: false,
    request: {
      model: "decora-jev",
      input: [
        {
          role: "user",
          content: [
            {
              type: "input_json",
              data: { image_url: "https://example.invalid/order.png", damaged: true },
            },
          ],
        },
      ],
      questions: [
        { type: "predicate", name: "damaged", instructions: "Is the package damaged?" },
      ],
    },
  },
];

let wasmExports = null;
let currentDialect = "openai";
let activePresetId = PRESETS[0].id;
let lastTranslateMeta = null;

const $ = (id) => document.getElementById(id);

function callWasm(cmd) {
  if (!wasmExports) {
    throw new Error("WASM module is not loaded yet.");
  }
  const { memory, alloc, dealloc, decora_eval } = wasmExports;
  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  const inputBytes = encoder.encode(JSON.stringify(cmd));
  const inPtr = alloc(inputBytes.length);
  new Uint8Array(memory.buffer, inPtr, inputBytes.length).set(inputBytes);

  const packed = decora_eval(inPtr, inputBytes.length);
  dealloc(inPtr, inputBytes.length);

  const outPtr = Number(packed >> 32n);
  const outLen = Number(packed & 0xffffffffn);
  const outStr = decoder.decode(new Uint8Array(memory.buffer, outPtr, outLen));
  dealloc(outPtr, outLen);
  return JSON.parse(outStr);
}

function pretty(obj) {
  return JSON.stringify(obj, null, 2);
}

function setDialect(dialect) {
  currentDialect = dialect;
  document.querySelectorAll("#dialectButtons .seg-btn").forEach((btn) => {
    btn.classList.toggle("active", btn.dataset.dialect === dialect);
  });
  $("inboundEndpointTag").textContent = DIALECT_ENDPOINTS[dialect] || dialect;
}

function setStatusPill(status) {
  const el = $("httpStatusPill");
  el.className = "http-pill";
  if (status === 200) {
    el.classList.add("http-200");
    el.textContent = "HTTP 200 OK";
  } else if (status >= 500) {
    el.classList.add("http-502");
    el.textContent = `HTTP ${status} BAD GATEWAY`;
  } else {
    el.classList.add("http-400");
    el.textContent = `HTTP ${status} BAD REQUEST`;
  }
}

function setModeBadge(mode, ok = true) {
  const el = $("modeBadge");
  el.className = "mode-badge";
  if (!ok) {
    el.classList.add("mode-error");
    el.textContent = "REJECTED PRE-FLIGHT";
  } else if (mode === "same_dialect_passthrough") {
    el.classList.add("mode-passthrough");
    el.textContent = "SAME-DIALECT PASSTHROUGH";
  } else {
    el.classList.add("mode-ir");
    el.textContent = "CROSS-DIALECT CANONICAL IR";
  }
}

function renderIrSummary(summary) {
  const el = $("irSummaryStrip");
  if (!summary) {
    el.innerHTML = `<span class="muted-note">No Canonical IR produced (request rejected during inbound validation).</span>`;
    return;
  }
  el.innerHTML = `
    <span class="mono-tag">alias: ${escapeHtml(summary.model_alias)}</span>
    <span class="mono-tag">questions: ${summary.question_count} (${escapeHtml(summary.question_types.join(", "))})</span>
    <span class="mono-tag">real images: ${summary.image_count}</span>
    <span class="mono-tag">cache: ${summary.has_prompt_cache}</span>
    <span class="mono-tag">safety_id: ${summary.has_safety_identifier}</span>
  `;
}

function renderLedger(status, clientBody, mode, stage) {
  const el = $("ledgerBox");
  const items = [];

  if (status !== 200) {
    const code = clientBody?.error?.code || "error";
    const msg = clientBody?.error?.message || "Rejected by Decora Rust validator.";
    items.push({
      cls: "ledger-fail",
      label: `Rejected at ${stage || "verification"} (${code})`,
      val: `HTTP ${status}`,
    });
    items.push({
      cls: "ledger-info",
      label: msg,
      val: "No retry / No normalization",
    });
  } else {
    items.push({
      cls: "ledger-pass",
      label: "All probabilities finite ∈ [0, 1] & tolerance ≤ 1e-4",
      val: "VERIFIED ✓",
    });

    // Compute displayed probability sum from verified response for visual proof
    const ans = clientBody?.answers;
    const ansList = Array.isArray(ans) ? ans : ans ? Object.values(ans) : [];
    for (const a of ansList) {
      if (a.type === "choice" && a.probabilities) {
        const probs = Array.isArray(a.probabilities)
          ? a.probabilities.map((p) => p.probability)
          : Object.values(a.probabilities);
        const sum = probs.reduce((acc, v) => acc + Number(v), 0);
        items.push({
          cls: "ledger-pass",
          label: `Choice distribution${a.name ? ` (${a.name})` : ""} · choice = ${JSON.stringify(a.choice)}`,
          val: `Σp = ${sum.toFixed(4)}`,
        });
      } else if (a.type === "score" && a.probabilities) {
        const probs = Array.isArray(a.probabilities)
          ? a.probabilities.map((p) => p.probability)
          : Object.values(a.probabilities);
        const sum = probs.reduce((acc, v) => acc + Number(v), 0);
        items.push({
          cls: "ledger-pass",
          label: `Score expectation${a.name ? ` (${a.name})` : ""} · S = Σ i·p_i`,
          val: `S = ${Number(a.score).toFixed(4)} (Σp = ${sum.toFixed(4)})`,
        });
      }
    }

    const decora = clientBody?.usage?.decora;
    if (decora) {
      items.push({
        cls: "ledger-info",
        label: `Usage Provenance (${decora.provider}: ${decora.input_tokens_basis})`,
        val: `details_complete: ${decora.details_complete}`,
      });
    } else {
      items.push({
        cls: "ledger-pass",
        label: mode === "same_dialect_passthrough"
          ? "Same-dialect native usage preserved untouched"
          : "Native OpenAI usage strictly verified",
        val: "zero usage.decora",
      });
    }
  }

  el.innerHTML = items
    .map(
      (it) =>
        `<div class="ledger-item ${it.cls}"><span>${escapeHtml(it.label)}</span><span class="ledger-val">${escapeHtml(it.val)}</span></div>`
    )
    .join("");
}

function renderFailoverTimeline(attempts) {
  const el = $("failoverTimeline");
  if (!attempts || attempts.length === 0) {
    el.classList.add("hidden");
    el.innerHTML = "";
    return;
  }
  el.classList.remove("hidden");
  el.innerHTML = attempts
    .map((a) => {
      const cls = a.simulated_status === 200 ? "attempt-ok" : "attempt-fail";
      const badge = a.mode === "same_dialect_passthrough" ? "PASSTHROUGH" : "CANONICAL IR";
      return `<div class="attempt-row ${cls}">
        <span>Attempt #${a.attempt} (${a.role} → ${a.target} · ${badge})</span>
        <span>HTTP ${a.simulated_status} · ${a.outcome}</span>
      </div>`;
    })
    .join("");
}

function escapeHtml(str) {
  return String(str)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function parseEditorJson(el) {
  try {
    return { ok: true, value: JSON.parse(el.value) };
  } catch (e) {
    return { ok: false, error: e.message };
  }
}

function runPipeline(phantom = false) {
  if (!wasmExports) return;

  const parsed = parseEditorJson($("requestEditor"));
  if (!parsed.ok) {
    setModeBadge("", false);
    setStatusPill(400);
    renderFailoverTimeline(null);
    renderIrSummary(null);
    $("wireHeaders").textContent = "Invalid JSON syntax in request editor.";
    $("wireOutput").textContent = `JSON Parse Error: ${parsed.error}`;
    $("upstreamEditor").value = "";
    const errBody = { error: { message: `Invalid JSON syntax: ${parsed.error}`, type: "invalid_request_error", param: null, code: "invalid_json" } };
    renderLedger(400, errBody, null, "json_parse");
    $("responseOutput").textContent = pretty(errBody);
    return;
  }

  const target = $("targetSelect").value;
  const usagePolicy = $("usagePolicySelect").value;
  const failover = $("failoverToggle").checked;
  const backup = $("backupSelect").value;

  const cmd = failover
    ? {
        op: "failover",
        dialect: currentDialect,
        target,
        backup,
        usage_policy: usagePolicy,
        request: parsed.value,
        phantom,
      }
    : {
        op: "translate",
        dialect: currentDialect,
        target,
        usage_policy: usagePolicy,
        request: parsed.value,
        phantom,
      };

  const res = callWasm(cmd);

  if (!res.ok) {
    lastTranslateMeta = null;
    setModeBadge("", false);
    setStatusPill(res.status || 400);
    renderFailoverTimeline(null);
    renderIrSummary(null);
    $("wireHeaders").textContent = `Blocked at stage: ${res.stage} (zero upstream network calls made)`;
    $("wireOutput").textContent = pretty(res.error_body);
    $("upstreamEditor").value = "// Blocked before contacting upstream";
    renderLedger(res.status || 400, res.error_body, null, res.stage);
    $("responseOutput").textContent = pretty(res.error_body);
    return;
  }

  lastTranslateMeta = {
    dialect: currentDialect,
    target: failover ? backup : target,
    usagePolicy,
    mode: res.mode,
  };

  setModeBadge(res.mode, true);
  renderFailoverTimeline(res.attempts || null);
  renderIrSummary(res.canonical_summary);

  $("wireHeaders").innerHTML = `
    <div>POST ${escapeHtml(res.target_endpoint)}</div>
    <div>${escapeHtml(res.auth_header)} · Content-Type: application/json</div>
  `;
  $("wireOutput").textContent = pretty(res.upstream_body);
  $("upstreamEditor").value = pretty(res.mock_upstream_response);

  setStatusPill(res.client_status);
  renderLedger(res.client_status, res.client_response, res.mode, res.verify_ok ? "ok" : "upstream_verify");
  $("responseOutput").textContent = pretty(res.client_response);
}

function reverifyEditedUpstream() {
  if (!wasmExports) return;
  const reqParsed = parseEditorJson($("requestEditor"));
  const upParsed = parseEditorJson($("upstreamEditor"));

  if (!reqParsed.ok || !upParsed.ok) {
    const errMsg = !reqParsed.ok ? `Request JSON error: ${reqParsed.error}` : `Upstream JSON error: ${upParsed.error}`;
    setStatusPill(400);
    const errBody = { error: { message: errMsg, type: "invalid_request_error", param: null, code: "invalid_json" } };
    renderLedger(400, errBody, null, "json_parse");
    $("responseOutput").textContent = pretty(errBody);
    return;
  }

  const failover = $("failoverToggle").checked;
  const effectiveTarget = failover ? $("backupSelect").value : $("targetSelect").value;
  const usagePolicy = $("usagePolicySelect").value;

  const res = callWasm({
    op: "verify",
    dialect: currentDialect,
    target: effectiveTarget,
    usage_policy: usagePolicy,
    request: reqParsed.value,
    upstream_response: upParsed.value,
  });

  if (res.ok) {
    setStatusPill(200);
    renderLedger(200, res.client_response, res.mode, "ok");
    $("responseOutput").textContent = pretty(res.client_response);
  } else {
    setStatusPill(res.status || 502);
    renderLedger(res.status || 502, res.error_body, lastTranslateMeta?.mode, res.stage);
    $("responseOutput").textContent = pretty(res.error_body);
  }
}

function injectPhantomAndVerify() {
  const parsed = parseEditorJson($("upstreamEditor"));
  if (!parsed.ok) {
    runPipeline(true);
    return;
  }
  const body = parsed.value;
  let mutated = false;
  const answers = Array.isArray(body.answers) ? body.answers : body.answers ? Object.values(body.answers) : [];
  for (const ans of answers) {
    if (ans.type === "choice" && ans.probabilities) {
      if (Array.isArray(ans.probabilities) && ans.probabilities.length > 0) {
        const last = ans.probabilities[ans.probabilities.length - 1];
        last.probability = Math.max(0, Math.round((last.probability - 0.07) * 10000) / 10000);
        mutated = true;
        break;
      } else if (typeof ans.probabilities === "object") {
        const keys = Object.keys(ans.probabilities);
        if (keys.length > 0) {
          const k = keys[keys.length - 1];
          ans.probabilities[k] = Math.max(0, Math.round((ans.probabilities[k] - 0.07) * 10000) / 10000);
          mutated = true;
          break;
        }
      }
    } else if (ans.type === "score" && ans.probabilities) {
      if (Array.isArray(ans.probabilities) && ans.probabilities.length > 0) {
        ans.probabilities[0].probability = Math.max(0, Math.round((ans.probabilities[0].probability - 0.07) * 10000) / 10000);
        mutated = true;
        break;
      } else if (typeof ans.probabilities === "object") {
        const keys = Object.keys(ans.probabilities);
        if (keys.length > 0) {
          ans.probabilities[keys[0]] = Math.max(0, Math.round((ans.probabilities[keys[0]] - 0.07) * 10000) / 10000);
          mutated = true;
          break;
        }
      }
    } else if (ans.type === "predicate" || ans.type === "noul") {
      const f = ans.type === "predicate" ? "probability" : "noul";
      ans[f] = 1.25;
      mutated = true;
      break;
    }
  }
  if (!mutated) {
    runPipeline(true);
    return;
  }
  $("upstreamEditor").value = pretty(body);
  reverifyEditedUpstream();
}

function loadPreset(preset) {
  activePresetId = preset.id;
  document.querySelectorAll(".preset-btn").forEach((btn) => {
    btn.classList.toggle("active", btn.dataset.presetId === preset.id);
  });
  setDialect(preset.dialect);
  $("targetSelect").value = preset.target;
  $("usagePolicySelect").value = preset.usagePolicy;
  $("failoverToggle").checked = preset.failover;
  $("backupSelect").disabled = !preset.failover;
  $("backupSelect").value = preset.backup;
  $("scenarioNote").textContent = preset.note;
  $("requestEditor").value = pretty(preset.request);
  runPipeline(preset.phantom);
}

function renderPresets() {
  const grid = $("presetGrid");
  grid.innerHTML = PRESETS.map(
    (p) => `
      <button type="button" class="preset-btn ${p.id === activePresetId ? "active" : ""}" data-preset-id="${p.id}">
        <span class="preset-title">${escapeHtml(p.title)}</span>
        <span class="preset-sub">${escapeHtml(p.sub)}</span>
      </button>
    `
  ).join("");
  grid.querySelectorAll(".preset-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      const preset = PRESETS.find((p) => p.id === btn.dataset.presetId);
      if (preset) loadPreset(preset);
    });
  });
}

function bindEvents() {
  document.querySelectorAll("#dialectButtons .seg-btn").forEach((btn) => {
    btn.addEventListener("click", () => {
      setDialect(btn.dataset.dialect);
      runPipeline(false);
    });
  });

  $("targetSelect").addEventListener("change", () => runPipeline(false));
  $("usagePolicySelect").addEventListener("change", () => runPipeline(false));
  $("backupSelect").addEventListener("change", () => runPipeline(false));
  $("failoverToggle").addEventListener("change", (e) => {
    $("backupSelect").disabled = !e.target.checked;
    runPipeline(false);
  });

  $("runBtn").addEventListener("click", () => runPipeline(false));
  $("reverifyBtn").addEventListener("click", reverifyEditedUpstream);
  $("injectPhantomBtn").addEventListener("click", injectPhantomAndVerify);

  $("formatReqBtn").addEventListener("click", () => {
    const parsed = parseEditorJson($("requestEditor"));
    if (parsed.ok) $("requestEditor").value = pretty(parsed.value);
  });

  const wireCopy = (btnId, getText) => {
    $(btnId).addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(getText());
        const btn = $(btnId);
        const prev = btn.textContent;
        btn.textContent = "Copied!";
        setTimeout(() => { btn.textContent = prev; }, 1100);
      } catch {
        // Ignore clipboard denial in restricted contexts.
      }
    });
  };
  wireCopy("copyReqBtn", () => $("requestEditor").value);
  wireCopy("copyWireBtn", () => $("wireOutput").textContent);
  wireCopy("copyRespBtn", () => $("responseOutput").textContent);
}

async function init() {
  renderPresets();
  bindEvents();

  const statusEl = $("wasmStatus");
  try {
    const resp = await fetch("./decora_wasm.wasm");
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    const bytes = await resp.arrayBuffer();
    const { instance } = await WebAssembly.instantiate(bytes);
    wasmExports = instance.exports;

    const kb = Math.round(bytes.byteLength / 1024);
    statusEl.className = "status-chip status-ready";
    statusEl.innerHTML = `<span class="dot"></span><span>Rust WASM Ready (${kb} KB)</span>`;

    loadPreset(PRESETS[0]);
  } catch (err) {
    statusEl.className = "status-chip status-error";
    statusEl.innerHTML = `<span class="dot"></span><span>WASM load failed: ${escapeHtml(err.message)}</span>`;
    $("responseOutput").textContent = `Failed to load ./decora_wasm.wasm: ${err.message}\nServe this directory over HTTP (e.g. Cloudflare Pages or python -m http.server).`;
  }
}

init();
