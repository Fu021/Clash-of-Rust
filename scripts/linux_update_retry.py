"""Back off between anonymous Release checks in Linux compatibility tests."""
import random
import time


RATE_LIMIT_MESSAGES = (
    'GitHub 暂时限制请求',
    'GitHub is temporarily rate limiting requests',
)


class ReleaseRateLimit(RuntimeError):
    pass


def retry_release_check(operation, step, checkpoint):
    delays = (60, 120, 240)
    for attempt in range(len(delays) + 1):
        checkpoint()
        try:
            return operation()
        except ReleaseRateLimit:
            if attempt == len(delays):
                raise
            delay = delays[attempt] + random.uniform(0, 30)
            print(f'{step}: GitHub rate limited the Release check; retry '
                  f'{attempt + 1}/{len(delays)} in {delay:.1f}s', flush=True)
            deadline = time.monotonic() + delay
            while (remaining := deadline - time.monotonic()) > 0:
                checkpoint()
                time.sleep(min(remaining, 1))
