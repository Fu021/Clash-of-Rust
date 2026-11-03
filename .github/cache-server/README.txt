GitHub Actions private cache
============================

This file is synchronized from the repository by the CI main-push workflow.
Do not edit it on the cache host: change .github/cache-server/README.txt and
let Actions update it after the change reaches main.

Host layout (paths relative to the configured cache root):
- bin/action_cache.py: installed cache service.
- config/server.json: service settings and token hash; keep private.
- data/: cache archives, grouped by rust, compiler, dependencies, tools and
  resources; data/.incoming holds incomplete uploads.
- state/index.sqlite: cache metadata and access order.
- state/README.txt: Actions-managed, atomically replaced README content.
- logs/server.log: rotating service log.
- README.txt: host-visible link to state/README.txt.

The service stores and transfers checked archives; compilation and tests run
on disposable GitHub runners. SHA-256 is checked on upload and restore. Each
group prefers its latest verified snapshot; older uploads cannot replace it.
The disk budget or minimum free space may evict entire groups, so missing
entries fall back to normal downloads or rebuilding. Limits are configured
in config/server.json.

GitHub-hosted Actions cache has a separate 9 GiB budget. Ubuntu 20.04 Python,
cloud images and verified guest DEB downloads use that GitHub cache; VM disks,
installed system state, application packages and test results are not cached.

Host maintenance (use the configured root and service name):
  CACHE_ROOT="$HOME/github_action_cache"  # adjust to the installed location
  systemctl --user status github-action-cache
  python3 "$CACHE_ROOT/bin/action_cache.py" status --root "$CACHE_ROOT"
  du -h --max-depth=2 "$CACHE_ROOT/data"

The status command may initialize directories or migrate the index. For a
read-only remote inventory, use the existing trusted CI artifact instead.
It does not verify files on the host or provide remote administration.

The service is confined to its cache data, state and log directories and does
not expose SSH or command execution. Keep tokens, configuration contents and
private addresses out of this file, repository commits and public logs.
For initial setup, upgrades and the synchronization contract, see
docs/workflows/cache-server.md; for inventory and cleanup, see
docs/workflows/cache-maintenance.md.
