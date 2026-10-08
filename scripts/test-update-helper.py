"""Test native update-helper failures without elevating or installing anything."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('executable', type=Path)
    executable = parser.parse_args().executable.resolve(strict=True)
    for failure in ('modified-package', 'deleted-package', 'malformed-plan'):
        with tempfile.TemporaryDirectory(prefix='clash update helper test ') as work:
            directory = Path(work)
            package = directory/'package with spaces'
            package.write_bytes(b'changed after download')
            plan = dict(package=str(package), executable=str(executable), sha256='0'*64, version='0.4.6', parent=0)
            data = json.dumps(plan) if failure != 'malformed-plan' else '{'
            (directory/'plan.json').write_text(data, encoding='utf-8')
            if failure == 'deleted-package':
                package.unlink()
            result = subprocess.run([executable, '--install-update', directory], capture_output=True, timeout=30)
            assert result.returncode == 0, (failure, result.returncode, result.stderr)
            outcome = json.loads((directory/'outcome.json').read_text(encoding='utf-8'))
            assert outcome['error'] and outcome['message'], failure
            assert not (directory/'ready').exists(), 'Invalid update reached installer handoff'
    print('PASS: native helper rejects modified/deleted packages and malformed plans before authorization or installation; paths with spaces supported')


if __name__ == '__main__':
    main()
