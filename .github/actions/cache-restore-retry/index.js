const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawn } = require("node:child_process");

const CACHE_ACTION = "55cc8345863c7cc4c66a329aec7e433d2d1c52a9";
const MAX_ATTEMPTS = 100;
const DEADLINE_MS = 20 * 60 * 1000;

function outputs(text) {
  const result = {};
  const lines = text.split(/\r?\n/);
  for (let index = 0; index < lines.length; index += 1) {
    const multiline = /^([^=<]+)<<(.+)$/.exec(lines[index]);
    if (multiline) {
      const value = [];
      while (++index < lines.length && lines[index] !== multiline[2]) {
        value.push(lines[index]);
      }
      if (index < lines.length) result[multiline[1]] = value.join("\n");
      continue;
    }
    const single = /^([^=]+)=(.*)$/.exec(lines[index]);
    if (single) result[single[1]] = single[2];
  }
  return result;
}

function outcome(code, log, values) {
  if (code === 0 && values["cache-matched-key"]) {
    return { kind: "hit", key: values["cache-matched-key"] };
  }
  const lines = log.split(/\r?\n/).filter((line) => !/^::debug::/i.test(line));
  const visible = lines.join("\n");
  if (/Cache restore skipped:.*does not permit reads/i.test(visible)) {
    return { kind: "permanent" };
  }
  if (/\[warning\]\s*An internal error has occurred in cache backend/i.test(visible)) {
    return { kind: "transient" };
  }
  const errors = lines.filter((line) =>
    /::(?:warning|error)::|failed to restore|timed?\s*out|ECONN|ENOTFOUND|EAI_AGAIN|socket hang up/i.test(line)
  ).join("\n");
  if (/cache read denied|permission denied|unauthori[sz]ed|forbidden|(?:^|\D)(?:401|403)(?:\D|$)|(?:path|key) validation error/i.test(errors)) {
    return { kind: "permanent" };
  }
  if (errors) {
    return { kind: "transient" };
  }
  if (code === 0 && /Cache not found for input keys:/i.test(visible)) {
    return { kind: "miss" };
  }
  return { kind: "permanent" };
}

async function restore(run, clean, wait, now, maxAttempts = MAX_ATTEMPTS, deadline = DEADLINE_MS) {
  const started = now();
  for (let attempt = 1; attempt <= maxAttempts && now() - started < deadline; attempt += 1) {
    console.log(`Cache lookup attempt ${attempt}/${maxAttempts}`);
    const lookup = await run(true);
    if (lookup.kind === "miss") return lookup;
    if (lookup.kind === "permanent") throw new Error("Cache lookup failed permanently");
    if (lookup.kind === "hit") {
      const restored = await run(false);
      if (restored.kind === "hit") return restored;
      await clean();
      if (restored.kind === "permanent") throw new Error("Cache restore failed permanently");
    }
    if (attempt < maxAttempts && now() - started < deadline) {
      await wait(Math.min(attempt * 1000, 5000));
    }
  }
  throw new Error("Cache restore retries exhausted; the build was stopped");
}

function runChild(entry, inputs, lookup) {
  return new Promise((resolve, reject) => {
    const directory = fs.mkdtempSync(path.join(os.tmpdir(), "cor-cache-retry-"));
    const output = path.join(directory, "output");
    fs.writeFileSync(output, "");
    const environment = {
      ...process.env,
      GITHUB_OUTPUT: output,
      INPUT_PATH: inputs.path,
      INPUT_KEY: inputs.key,
      "INPUT_RESTORE-KEYS": inputs.restoreKeys,
      "INPUT_LOOKUP-ONLY": String(lookup),
      "INPUT_FAIL-ON-CACHE-MISS": "false",
      INPUT_ENABLECROSSOSARCHIVE: "false",
    };
    const child = spawn(process.execPath, [entry], { env: environment });
    let log = "";
    let failed = false;
    const record = (chunk, stream) => {
      const text = chunk.toString();
      stream.write(text);
      log = (log + text).slice(-262144);
    };
    child.stdout.on("data", (chunk) => record(chunk, process.stdout));
    child.stderr.on("data", (chunk) => record(chunk, process.stderr));
    child.on("error", (error) => {
      failed = true;
      fs.rmSync(directory, { recursive: true, force: true });
      reject(error);
    });
    child.on("close", (code) => {
      if (failed) return;
      try {
        const values = outputs(fs.readFileSync(output, "utf8"));
        resolve(outcome(code, log, values));
      } catch (error) {
        reject(error);
      } finally {
        fs.rmSync(directory, { recursive: true, force: true });
      }
    });
  });
}

async function clean(scope) {
  const workspace = process.env.GITHUB_WORKSPACE;
  const temporary = process.env.RUNNER_TEMP;
  if (!workspace || !temporary || process.env.GITHUB_ACTIONS !== "true") {
    throw new Error("Cache cleanup is restricted to Actions runners");
  }
  if (scope === "compiler") {
    await new Promise((resolve, reject) => {
      const child = spawn("python", ["-c", "import sys; sys.path.insert(0, 'scripts'); from action_cache import discard_restore; discard_restore('.')"], {
        cwd: workspace,
        stdio: "inherit",
        env: process.env,
      });
      child.on("error", reject);
      child.on("close", (code) => code === 0 ? resolve() : reject(new Error("Compiler cache cleanup failed")));
    });
  } else {
    const paths = {
      maven: path.join(temporary, "cor-android-gradle", "caches", "modules-2"),
      go: path.join(workspace, ".go-build"),
      container: path.join(temporary, "cor-linux-build-cache"),
    };
    if (!paths[scope]) throw new Error("Unknown cache cleanup scope");
    fs.rmSync(paths[scope], { recursive: true, force: true, maxRetries: 3 });
  }
}

async function main() {
  const inputs = {
    path: process.env.INPUT_PATH,
    key: process.env.INPUT_KEY,
    restoreKeys: process.env["INPUT_RESTORE-KEYS"] || "",
  };
  if (!inputs.path || !inputs.key || !process.env.GITHUB_OUTPUT || !process.env.RUNNER_WORKSPACE) {
    throw new Error("Missing cache inputs or Actions runner environment");
  }
  if (/[\r\n]/.test(inputs.key)) throw new Error("Invalid primary cache key");
  const entry = path.join(process.env.RUNNER_WORKSPACE, "..", "_actions", "actions", "cache", CACHE_ACTION, "dist", "restore-only", "index.js");
  if (!fs.existsSync(entry)) throw new Error("Pinned cache action is unavailable");
  const result = await restore(
    (lookup) => runChild(entry, inputs, lookup),
    () => clean(process.env.INPUT_CLEANUP),
    (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
    Date.now,
  );
  const hit = result.kind === "hit" && result.key.toLowerCase() === inputs.key.toLowerCase();
  const lines = [`cache-primary-key=${inputs.key}`, `cache-hit=${hit}`];
  if (result.kind === "hit") {
    if (/[\r\n]/.test(result.key)) throw new Error("Invalid matched cache key");
    lines.push(`cache-matched-key=${result.key}`);
  }
  fs.appendFileSync(process.env.GITHUB_OUTPUT, `${lines.join("\n")}\n`);
  console.log(result.kind === "hit" ? "Cache restored successfully." : "Cache is absent; continuing with a cold build.");
}

module.exports = { CACHE_ACTION, outcome, outputs, restore, runChild };
if (require.main === module) {
  main().catch((error) => {
    console.error(`::error::${error.message}`);
    process.exitCode = 1;
  });
}
