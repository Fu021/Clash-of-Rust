// No npm dependencies: this action only drives the repository's Python client.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawnSync } = require('node:child_process');

const input = (name, fallback = '') => process.env[`INPUT_${name.toUpperCase()}`] || fallback;
const post = process.env.STATE_cache_state;
let state;
if (post) {
  state = post;
} else {
  state = path.join(process.env.RUNNER_TEMP, `private-cache-${crypto.randomUUID()}.json`);
  const config = {
    url: input('url'), token: input('token'),
    proxy: input('proxy', process.platform === 'win32' ? '' : 'http://127.0.0.1:19082'),
    group: input('group'),
    key: input('key'), base_key: input('key'),
    prefix: input('restore-keys').split('\n').map(s => s.trim()).filter(Boolean),
    paths: input('paths'), kind: input('kind', 'archives'),
    source: path.resolve(input('source', '.')), save: input('save', 'true') === 'true',
    restore_seconds: Number(input('restore-timeout', '300')),
    github_hit: input('github-hit'),
    github_failed: input('github-failed', 'false') === 'true',
    immutable: input('immutable', 'false') === 'true',
    runtime: input('runtime'),
  };
  if (!Number.isInteger(config.restore_seconds) || config.restore_seconds < 30 || config.restore_seconds > 900) {
    throw new Error('Invalid private cache restore budget');
  }
  fs.writeFileSync(state, JSON.stringify(config), { mode: 0o600 });
  fs.appendFileSync(process.env.GITHUB_STATE, `cache_state=${state}\n`);
}
const script = path.resolve(__dirname, '../../../scripts/action_cache.py');
const config = JSON.parse(fs.readFileSync(state, 'utf8'));
const runtime = (operation) => {
  if (!config.runtime || config.runtime === 'apt') return;
  if (!['bootstrap', 'rust', 'go', 'java', 'android', 'android-package', 'emulator'].includes(config.runtime)) {
    throw new Error('Invalid runtime cache category');
  }
  if (path.resolve(config.paths) !== path.join(process.env.RUNNER_TEMP, `cor-runtime-${config.runtime}`)) {
    throw new Error('Runtime cache must use its dedicated temporary directory');
  }
  const snapshot = path.resolve(__dirname, '../../../scripts/runtime_cache.py');
  const result = spawnSync('python', [snapshot, operation, '--kind', config.runtime, '--directory', config.paths], {
    stdio: 'inherit', timeout: 4 * 60 * 1000,
  });
  if (result.error || result.status !== 0) {
    console.log(`::warning::Runtime cache ${operation} unavailable; standard tool installation remains enabled.`);
    return false;
  }
  return true;
};
if (post && config.save && runtime('capture') === false) {
  fs.rmSync(config.paths, { force: true, recursive: true });
}
// Cache writes are optional. A slow relay must not turn a tested build into
// a workflow timeout; incomplete uploads never replace the previous snapshot.
const result = spawnSync('python', [script, post ? 'save' : 'restore', '--state', state], {
  // Python keeps fully verified shards after its soft restore deadline. The
  // watchdog also bounds a blocked extractor; then discard every partial
  // compiler object before allowing normal compilation to continue.
  stdio: 'inherit', timeout: post ? 5 * 60 * 1000
    : (JSON.parse(fs.readFileSync(state, 'utf8')).restore_seconds + 90) * 1000,
});
if (!post && (result.error || result.status !== 0)) {
  console.log('::warning::Private cache restore interrupted; discarding partial compilation objects before building.');
  const cleanup = spawnSync('python', [script, 'discard-restore', '--state', state], {
    stdio: 'inherit', timeout: 2 * 60 * 1000,
  });
  if (cleanup.error || cleanup.status !== 0) {
    throw new Error('Cannot safely clean an interrupted private cache restore');
  }
} else if (post && result.error?.code === 'ETIMEDOUT') {
  const message = 'Private cache save exceeded 5 minutes; completed shards remain reusable and incomplete uploads are discarded.';
  console.log(`::warning::${message}`);
  if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, `\n${message}\n`);
} else if (result.status !== 0) {
  console.log('::warning::Private cache helper failed; normal compilation remains enabled.');
}
if (!post) {
  runtime('hydrate');
  const config = JSON.parse(fs.readFileSync(state, 'utf8'));
  fs.appendFileSync(process.env.GITHUB_OUTPUT, `state=${state}\n`);
  for (const [name, field] of [['cache-hit', 'hit'], ['restored', 'restored'], ['available', 'available']]) {
    fs.appendFileSync(process.env.GITHUB_OUTPUT, `${name}=${config[field] === true}\n`);
  }
} else {
  fs.rmSync(state, { force: true });
}
