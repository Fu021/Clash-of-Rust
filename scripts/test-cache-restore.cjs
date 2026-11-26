const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const test = require("node:test");
const { CACHE_ACTION, outcome, outputs, restore, runChild } = require("../.github/actions/cache-restore-retry/index.js");

test("each retry job materializes the pinned upstream cache action", () => {
  const workflow = fs.readFileSync(path.join(__dirname, "../.github/workflows/ci.yml"), "utf8");
  const jobs = workflow.split(/^  (?=[a-z_]+:\s*$)/m);
  for (const name of ["android_maven", "android", "check"]) {
    const job = jobs.find((part) => part.startsWith(`${name}:\n`));
    assert.ok(job, `${name} job is missing`);
    const retry = job.indexOf("uses: ./.github/actions/cache-restore-retry");
    const upstream = job.indexOf(`actions/cache/restore@${CACHE_ACTION}`);
    assert.ok(upstream >= 0 && upstream < retry, `${name} must prepare the pinned action before retrying`);
  }
  const container = fs.readFileSync(path.join(__dirname, "../.github/actions/linux-build-cache/action.yml"), "utf8");
  assert.ok(container.includes("uses: ./.github/actions/cache-restore-retry"));
  assert.ok(container.includes(`actions/cache/save@${CACHE_ACTION}`));
});

test("compiler caches are queried before the one retryable restore", () => {
  const workflow = fs.readFileSync(path.join(__dirname, "../.github/workflows/ci.yml"), "utf8");
  const jobs = workflow.split(/^  (?=[a-z_]+:\s*$)/m);
  for (const name of ["android", "check"]) {
    const job = jobs.find((part) => part.startsWith(`${name}:\n`));
    const rustCache = job.split("uses: Swatinem/rust-cache@")[1].split("\n      - ")[0];
    assert.match(rustCache, /lookup-only: true/);
    assert.match(job, /name: Restore (?:Android|compatible native) workspace with retries/);
  }
});

test("native cache save and retry restore use the same key", () => {
  const workflow = fs.readFileSync(path.join(__dirname, "../.github/workflows/ci.yml"), "utf8");
  const check = workflow.split(/^  (?=[a-z_]+:\s*$)/m).find((part) => part.startsWith("check:\n"));
  const save = check.split("uses: Swatinem/rust-cache@")[1].split("\n      - ")[0];
  const restore = check.split("name: Restore compatible native workspace with retries")[1].split("\n      - ")[0];
  const cacheKey = save.match(/^\s+key: (.+)$/m)[1];
  const restoreKey = restore.match(/^\s+key: (.+)$/m)[1];
  assert.equal(restoreKey, `\${{ steps.rust_identity.outputs.prefix }}-${cacheKey}`);
});

test("cache outputs and missed, failed, or completed restores remain distinct", () => {
  assert.deepEqual(outputs("cache-primary-key<<eof\nprimary\neof\ncache-matched-key=found\n"), {
    "cache-primary-key": "primary",
    "cache-matched-key": "found",
  });
  assert.equal(outcome(0, "Cache not found for input keys: x", {}).kind, "miss");
  assert.equal(outcome(0, "Cache not found for input keys: a403b-a503b", {}).kind, "miss");
  assert.equal(outcome(0, '::debug::Headers: {"keep-alive":"timeout=5"}\nCache not found for input keys: x', {}).kind, "miss");
  assert.equal(outcome(0, "::warning::Failed to restore: HTTP 503\nCache not found for input keys: x", {}).kind, "transient");
  assert.equal(outcome(0, "::warning::Failed to restore: HTTP 403", {}).kind, "permanent");
  assert.equal(outcome(0, "[warning]An internal error has occurred in cache backend.", {}).kind, "transient");
  assert.equal(outcome(0, "Cache restore skipped: the effective cache-mode 'off' does not permit reads.\nCache not found for input keys: x", {}).kind, "permanent");
  assert.equal(outcome(0, "Cache restored successfully", {}).kind, "permanent");
  assert.deepEqual(outcome(0, "Cache restored successfully", { "cache-matched-key": "x" }), { kind: "hit", key: "x" });
});

test("a genuine miss does not download or retry", async () => {
  const calls = [];
  const result = await restore(async (lookup) => {
    calls.push(lookup);
    return { kind: "miss" };
  }, async () => assert.fail("unexpected cleanup"), async () => {}, () => 0);
  assert.equal(result.kind, "miss");
  assert.deepEqual(calls, [true]);
});

test("temporary lookup and download failures retry after partial cleanup", async () => {
  const results = [
    { kind: "transient" }, { kind: "hit", key: "old" }, { kind: "transient" },
    { kind: "hit", key: "old" }, { kind: "hit", key: "old" },
  ];
  const calls = [];
  let cleanups = 0;
  const result = await restore(async (lookup) => {
    calls.push(lookup);
    return results.shift();
  }, async () => { cleanups += 1; }, async () => {}, () => 0);
  assert.equal(result.key, "old");
  assert.deepEqual(calls, [true, true, false, true, false]);
  assert.equal(cleanups, 1);
});

test("persistent errors stop the build and permanent restore failures clean partial data", async () => {
  let calls = 0;
  await assert.rejects(restore(async () => {
    calls += 1;
    return { kind: "transient" };
  }, async () => {}, async () => {}, () => 0, 3), /retries exhausted/);
  assert.equal(calls, 3);
  let cleanups = 0;
  await assert.rejects(restore(async (lookup) => ({ kind: lookup ? "hit" : "permanent" }),
    async () => { cleanups += 1; }, async () => {}, () => 0), /failed permanently/);
  assert.equal(cleanups, 1);
});

test("retry deadline prevents starting another lookup", async () => {
  let clock = 0;
  let calls = 0;
  await assert.rejects(restore(async () => {
    calls += 1;
    return { kind: "transient" };
  }, async () => {}, async (milliseconds) => { clock += milliseconds; }, () => clock, 100, 2001),
  /retries exhausted/);
  assert.equal(calls, 2);
});

test("a cache deleted during download is cleaned before a confirmed miss", async () => {
  const results = [{ kind: "hit", key: "old" }, { kind: "miss" }, { kind: "miss" }];
  let cleanups = 0;
  const result = await restore(async () => results.shift(),
    async () => { cleanups += 1; }, async () => {}, () => 0);
  assert.equal(result.kind, "miss");
  assert.equal(cleanups, 1);
});

test("the pinned-action process receives inputs and returns its actual matched key", async () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "cor-cache-test-"));
  const entry = path.join(directory, "fixture.cjs");
  try {
    fs.writeFileSync(entry, `
      const fs = require("node:fs");
      if (process.env.INPUT_PATH !== "cache-path" || process.env.INPUT_KEY !== "primary") process.exit(1);
      if (process.env["INPUT_LOOKUP-ONLY"] === "true") {
        fs.appendFileSync(process.env.GITHUB_OUTPUT, "cache-matched-key<<end\\nmatching-key\\nend\\n");
      } else {
        console.log("Failed to restore: temporary network error");
        console.log("Cache not found for input keys: primary");
      }
    `);
    const inputs = { path: "cache-path", key: "primary", restoreKeys: "" };
    assert.deepEqual(await runChild(entry, inputs, true), { kind: "hit", key: "matching-key" });
    assert.equal((await runChild(entry, inputs, false)).kind, "transient");
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});

test("the action locates the pinned cache in a runner workspace and exports a miss", () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "cor-cache-runner-"));
  const runnerWorkspace = path.join(directory, "work", "repo");
  const workspace = path.join(runnerWorkspace, "repo");
  const entry = path.join(directory, "work", "_actions", "actions", "cache", CACHE_ACTION,
    "dist", "restore-only", "index.js");
  const output = path.join(directory, "output");
  try {
    fs.mkdirSync(path.dirname(entry), { recursive: true });
    fs.mkdirSync(workspace, { recursive: true });
    fs.writeFileSync(entry, 'console.log("Cache not found for input keys: primary");\n');
    fs.writeFileSync(output, "");
    const result = spawnSync(process.execPath, [path.join(__dirname, "../.github/actions/cache-restore-retry/index.js")], {
      env: {
        ...process.env,
        GITHUB_ACTIONS: "true",
        GITHUB_OUTPUT: output,
        GITHUB_WORKSPACE: workspace,
        RUNNER_WORKSPACE: runnerWorkspace,
        INPUT_PATH: "cache-path",
        INPUT_KEY: "primary",
        INPUT_CLEANUP: "go",
      },
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(outputs(fs.readFileSync(output, "utf8")), {
      "cache-primary-key": "primary",
      "cache-hit": "false",
    });
  } finally {
    fs.rmSync(directory, { recursive: true, force: true });
  }
});
