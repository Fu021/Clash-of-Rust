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
  };
  fs.writeFileSync(state, JSON.stringify(config), { mode: 0o600 });
  fs.appendFileSync(process.env.GITHUB_STATE, `cache_state=${state}\n`);
}
const script = path.resolve(__dirname, '../../../scripts/action_cache.py');
const result = spawnSync('python', [script, post ? 'save' : 'restore', '--state', state], { stdio: 'inherit' });
if (result.status !== 0) {
  console.log('::warning::Private cache helper failed; normal compilation remains enabled.');
}
if (!post) {
  const config = JSON.parse(fs.readFileSync(state, 'utf8'));
  for (const [name, field] of [['cache-hit', 'hit'], ['restored', 'restored'], ['available', 'available']]) {
    fs.appendFileSync(process.env.GITHUB_OUTPUT, `${name}=${config[field] === true}\n`);
  }
} else {
  fs.rmSync(state, { force: true });
}
