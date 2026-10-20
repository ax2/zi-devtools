const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");

const root = path.resolve(__dirname, "..");
const html = fs.readFileSync(path.join(root, "views/main.html"), "utf8");
const script = html.match(/<script>([\s\S]*?)<\/script>/)?.[1];
assert.ok(script, "self-contained view script exists");
assert.doesNotMatch(script, /\bfetch\s*\(|XMLHttpRequest|WebSocket|http:\/\//, "View must not implement networking");
assert.doesNotMatch(script, /grantId|x-zicode-desktop-token|headers\s*:|dnsResult/i, "View must not access Host secrets or DNS results");
const fixture = JSON.parse(fs.readFileSync(path.join(root, "fixtures/http-get-view-cases.json"), "utf8"));

class Element {
  constructor(id) {
    this.id = id;
    this.value = id === "origin" ? "https://service.example.test" : id === "endpoint" ? "health" : "";
    this.textContent = "";
    this.dataset = {};
    this.hidden = false;
    this.disabled = false;
    this.listeners = new Map();
  }
  addEventListener(name, callback) { this.listeners.set(name, callback); }
  focus() {}
  async emit(name) { return this.listeners.get(name)?.(); }
}

async function makeHarness(getApi) {
  const elements = new Map();
  const document = { getElementById(id) { if (!elements.has(id)) elements.set(id, new Element(id)); return elements.get(id); } };
  const sdk = { ready: async () => {}, context: { get: async () => ({}) }, ...(getApi === undefined ? {} : { services: { http: { get: getApi } } }) };
  const window = { zicode: sdk };
  vm.runInNewContext(script, { window, document, URL, TextEncoder, AbortController, Object, Error, console });
  await new Promise(setImmediate);
  return { elements, sdk, get: id => document.getElementById(id), click: id => document.getElementById(id).emit("click") };
}

async function successCase(row) {
  const serialized = JSON.stringify(row.data);
  assert.equal(Buffer.byteLength(serialized), row.byteLength, `${row.id} byteLength fixture`);
  let calls = 0;
  const harness = await makeHarness(async (request, options) => {
    calls++;
    assert.deepEqual({ ...request }, { origin: "https://service.example.test", endpoint: row.endpoint });
    assert.deepEqual(Object.keys(options), ["signal"]);
    return { schemaVersion: 1, operation: "http.get", result: { endpoint: row.endpoint, status: row.httpStatus, contentType: row.contentType, data: row.data, byteLength: row.byteLength } };
  });
  harness.get("endpoint").value = row.endpoint;
  await harness.click("run");
  assert.equal(calls, 1);
  assert.equal(harness.get("summary").hidden, false);
  assert.equal(harness.get("result-status").textContent, String(row.httpStatus));
  assert.equal(harness.get("result-size").textContent, `${row.byteLength} 字节`);
  if (row.endpoint === "health") assert.equal(harness.get("result-health").textContent, row.data.status);
  assert.doesNotMatch([
    harness.get("result-endpoint").textContent,
    harness.get("result-status").textContent,
    harness.get("result-size").textContent,
    harness.get("result-health").textContent,
    harness.get("result-fields").textContent,
  ].join(" "), /service\.example|sample|jvm\.memory|components/);
  const status = harness.get("status").textContent;
  if (row.httpStatus === 503) assert.match(status, /503.*诊断数据/);
  else assert.match(status, /仅展示摘要/);
}

async function main() {
  const success = fixture.cases.filter(row => row.httpStatus);
  for (const row of success) await successCase(row);

  const declined = fixture.cases.find(row => row.id === "user-declined");
  let networkRequests = 0;
  const declinedHarness = await makeHarness(async () => {
    if (declined.confirmed) networkRequests++;
    return declined.response;
  });
  await declinedHarness.click("run");
  assert.equal(networkRequests, 0);
  assert.match(declinedHarness.get("status").textContent, /拒绝了本次请求/);

  const mismatch = await makeHarness(undefined);
  await mismatch.click("run");
  assert.match(mismatch.get("status").textContent, /尚未提供/);

  for (const row of fixture.cases.filter(item => item.errorCode && item.id !== "cancelled")) {
    let requests = 0;
    const h = await makeHarness(async () => {
      requests += row.networkRequests ?? 0;
      const error = new Error("RAW_NETWORK_ERROR_MUST_NOT_APPEAR");
      error.code = row.errorCode;
      throw error;
    });
    h.get("endpoint").value = row.endpoint;
    await h.click("run");
    assert.ok(h.get("status").textContent.length > 0, row.id);
    assert.doesNotMatch(h.get("status").textContent, /RAW_NETWORK_ERROR/);
    assert.equal(requests, row.networkRequests ?? 0, row.id);
  }

  for (const row of fixture.canonicalOriginVectors) {
    let calls = 0;
    const h = await makeHarness(async request => {
      calls++;
      assert.equal(request.origin, row.expected, row.id);
      return { schemaVersion: 1, operation: "http.get", status: "declined" };
    });
    h.get("origin").value = row.input;
    await h.click("run");
    assert.equal(calls, 1, row.id);
  }

  for (const row of fixture.securityVectors) {
    let calls = 0;
    const h = await makeHarness(async () => { calls++; return {}; });
    if (row.origin) h.get("origin").value = row.origin;
    if (row.endpoint) h.get("endpoint").value = row.endpoint;
    await h.click("run");
    assert.equal(calls, 0, row.id);
    if (row.id === "endpoint-outside-allowlist") assert.match(h.get("status").textContent, /端点/);
    else assert.ok(h.get("status").textContent.length > 0, row.id);
  }

  const cancelled = fixture.cases.find(row => row.id === "cancelled");
  let finishCommit;
  let beganCommit;
  const commitStarted = new Promise(resolve => { beganCommit = resolve; });
  const cancelHarness = await makeHarness((_request, { signal }) => {
    beganCommit();
    return new Promise((_resolve, reject) => {
      signal.addEventListener("abort", () => { finishCommit = () => { const error = new Error("ignored"); error.code = cancelled.errorCode; reject(error); }; });
    });
  });
  const pending = cancelHarness.click("run");
  await commitStarted;
  await cancelHarness.click("cancel");
  assert.equal(cancelHarness.get("run").disabled, true, "run remains blocked until commit finality");
  assert.equal(cancelHarness.get("cancel").disabled, true, "do not send duplicate cancel while final state is pending");
  assert.match(cancelHarness.get("status").textContent, /等待.*最终状态/);
  assert.ok(finishCommit, "AbortSignal reached Host bridge");
  finishCommit();
  await pending;
  assert.match(cancelHarness.get("status").textContent, /请求已取消/);
  assert.equal(cancelHarness.get("run").disabled, false, "run becomes available after final state");

  console.log(`PASS ${success.length} success, declined/profile, ${fixture.cases.filter(item => item.errorCode && item.id !== "cancelled").length} errors, ${fixture.securityVectors.length} input guards, ${fixture.canonicalOriginVectors.length} canonical-origin cases, cancel waits for commit finality`);
}

main().catch(error => { console.error(error); process.exitCode = 1; });
