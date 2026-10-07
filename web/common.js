// shared by the fnative pages: the token (from ?token=..., kept for this tab) and API calls
const TOKEN = (() => {
  const q = new URLSearchParams(location.search).get("token");
  try { if (q) sessionStorage.setItem("fnative-token", q); return q || sessionStorage.getItem("fnative-token") || ""; }
  catch { return q || ""; }
})();

async function api(method, path, body) {
  const r = await fetch(path, {
    method, headers: { Authorization: "Bearer " + TOKEN, "Content-Type": "application/json" },
    body: body === undefined ? undefined : (typeof body === "string" ? body : JSON.stringify(body)),
  });
  const j = await r.json().catch(() => ({ error: r.statusText }));
  if (!r.ok) throw new Error(j.error || r.statusText);
  return j;
}
const game = (iface, fn, ...args) => api("POST", "/api/game", { interface: iface, function: fn, args }).then((j) => j.result);
const native = (plugin, fn, input = "") => api("POST", `/api/native/${plugin}/${fn}`, input).then((j) => j.result);
const $ = (id) => document.getElementById(id);
const esc = (s) => String(s ?? "").replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));

function needToken() {
  if (TOKEN) return false;
  document.body.innerHTML = `<main class="wrap"><h1><b>f</b>native</h1><p>Open this page with the token:
    <code>http://127.0.0.1:8790/?token=&lt;contents of web-token.txt beside factorio-native.exe&gt;</code></p></main>`;
  return true;
}
