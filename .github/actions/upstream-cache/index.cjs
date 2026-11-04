const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawnSync } = require('node:child_process');

if (process.env.STATE_original_download_config) {
  const script = path.resolve(__dirname, '../../../scripts/upstream_cache.py');
  if (process.env.COR_UPSTREAM_REPORT_ATTEMPTED !== 'true') {
    const result = spawnSync('python', [script, 'submit'], {
      stdio: 'inherit', timeout: 90 * 1000,
    });
    if (result.error || result.status !== 0) {
      console.log('Original download manifest unavailable; no runner archive upload is attempted.');
    }
  }
  fs.rmSync(process.env.STATE_original_download_config, { force: true });
} else {
  const state = path.join(process.env.RUNNER_TEMP, `original-download-${crypto.randomUUID()}.json`);
  fs.writeFileSync(state, JSON.stringify({
    url: process.env.INPUT_URL || '', token: process.env.INPUT_TOKEN || '',
    proxy: process.platform === 'win32' ? '' : 'http://127.0.0.1:19082',
  }), { mode: 0o600 });
  fs.appendFileSync(process.env.GITHUB_STATE, `original_download_config=${state}\n`);
  fs.appendFileSync(process.env.GITHUB_ENV, `COR_UPSTREAM_CACHE_CONFIG=${state}\n`);
  const apt = path.join(process.env.RUNNER_TEMP, 'cor-host-apt');
  fs.appendFileSync(process.env.GITHUB_ENV, `COR_HOST_APT_CACHE=${apt}\n`);
  if (!process.env.COR_LINUX_APT_CACHE) {
    fs.appendFileSync(process.env.GITHUB_ENV, `COR_LINUX_APT_CACHE=${apt}\n`);
  }
}
