"""Headless terminal diagnostics and deterministic real-browser region collection."""
import argparse
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import os
import re
from pathlib import Path
import shutil
import subprocess
import tempfile
import threading
import time

from build_support import ROOT, write_json


def receiver(browser=False, dns=True):
    reports = []
    token = 'c' * 32
    policy = re.search(r'pub const CSP: &str =\s*"([^"]+)";',
                       (ROOT/'src/claude_check/browser.rs').read_text()).group(1)
    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_): pass
        def reply(self, code, data, kind='application/json'):
            self.send_response(code)
            self.send_header('Content-Type', kind)
            self.send_header('Content-Length', str(len(data)))
            self.send_header('Connection', 'close')
            self.send_header('Content-Security-Policy', policy)
            self.end_headers()
            self.wfile.write(data)
        def do_GET(self):
            name = self.path.removeprefix('/'+token+'/')
            if browser and name in ('', 'browser.js', 'browser.css'):
                file = ROOT/'resources/claude-check'/(name or 'browser.html')
                kind = 'text/html' if not name else 'text/javascript' if name.endswith('.js') else 'text/css'
                self.reply(200, file.read_bytes(), kind+'; charset=utf-8')
            else: self.reply(404, b'{}')
        def do_POST(self):
            if self.headers.get('X-Clash-Diagnostic') != '1':
                self.reply(403, b'{}'); return
            if self.path == '/'+token+'/dns/start':
                self.reply(200 if dns else 503, b'{"id":"fixture123"}' if dns else b'{}'); return
            if self.path != '/'+token+'/report':
                self.reply(404, b'{}'); return
            length = int(self.headers.get('Content-Length', '0'))
            assert 0 < length <= 64*1024
            reports.append(json.loads(self.rfile.read(length)))
            self.reply(200, b'{"ok":true}')
    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server, f'http://127.0.0.1:{server.server_port}/{token}/', reports


def terminal(executable):
    server, root, reports = receiver()
    try:
        with tempfile.TemporaryDirectory(prefix='clash-code-diagnostic-') as temporary:
            directory = Path(temporary)
            data = directory/'must-not-initialize'
            output = directory/'report.json'
            env = dict(os.environ, CLASH_OF_RUST_DATA_DIR=str(data),
                       HTTPS_PROXY='http://fixture-user:fixture-password@127.0.0.1:7890/private?token=fixture-password',
                       ANTHROPIC_API_KEY='fixture-api-secret-never-export',
                       ANTHROPIC_BASE_URL='https://api.anthropic.com/private?token=fixture-api-secret-never-export')
            subprocess.run([str(executable), '--claude-diagnose', '--offline', '--output', str(output),
                            '--submit', root+'report'], env=env, check=True, timeout=30)
            report = json.loads(output.read_text(encoding='utf-8'))
            assert report['schema'] == 1 and report['environment'] == 'code'
            assert reports == [report], 'Terminal report must reach the loopback receiver unchanged'
            assert not data.exists(), 'Headless detection must not initialize GUI/tray/core/store'
            serialized = json.dumps(report)
            for secret in ('fixture-user', 'fixture-password', 'fixture-api-secret-never-export'):
                assert secret not in serialized, 'Diagnostic report exported credentials'
            return {'status': 'passed', 'headless': True, 'callback_round_trip': True, 'credentials_redacted': True}
    finally:
        server.shutdown(); server.server_close()


RTC_FIXTURE = """
(() => {
window.clashRtcClosed = 0;
window.clashLocationRequests = 0;
const locate = navigator.geolocation.getCurrentPosition.bind(navigator.geolocation);
navigator.geolocation.getCurrentPosition = (...args) => { window.clashLocationRequests++; return locate(...args); };
class FixturePeer {
    createDataChannel() {}
    async createOffer() { return {type:'offer',sdp:''}; }
    async setLocalDescription() {
        if (window.clashRtcMode === 'timeout') return;
        queueMicrotask(() => {
            for (const address of ['8.8.4.4','2606:4700:4700::1111','192.168.1.2','fixture.local']) {
                this.onicecandidate({candidate:{address,candidate:'candidate:fixture'}});
            }
            this.onicecandidate({candidate:null});
        });
    }
    close() { window.clashRtcClosed++; }
}
window.RTCPeerConnection = window.clashRtcMode === 'absent' ? undefined : FixturePeer;
})();
"""


def browsers(output):
    from playwright.sync_api import sync_playwright
    chrome = next((shutil.which(name) for name in ('google-chrome', 'google-chrome-stable', 'chromium') if shutil.which(name)), None)
    if not chrome:
        raise RuntimeError('Install Chromium/Chrome before running the browser regression suite')
    cases = []
    scenarios = [('complete', 'en-US', 'America/New_York'),
                 ('absent', 'zh-CN', 'Asia/Shanghai'),
                 ('timeout', 'zh-TW', 'Asia/Taipei')]
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(executable_path=chrome, headless=True)
        try:
            for mode, locale, timezone in scenarios:
                server, root, reports = receiver(browser=True, dns=mode != 'timeout')
                context = browser.new_context(locale=locale, timezone_id=timezone,
                                              geolocation={'latitude': 37.7, 'longitude': -122.4})
                failures = []
                images = []
                context.add_init_script(f"window.clashRtcMode={json.dumps(mode)};\n"+RTC_FIXTURE)
                def external(route):
                    url = route.request.url
                    if url.startswith(root): route.continue_(); return
                    if url.startswith('https://api.ipify.org/') and mode != 'timeout':
                        route.fulfill(status=200, content_type='application/json', headers={'Access-Control-Allow-Origin':'*'}, body='{"ip":"8.8.8.8"}'); return
                    if url.startswith('https://api6.ipify.org/') and mode == 'complete':
                        route.fulfill(status=200, content_type='application/json', headers={'Access-Control-Allow-Origin':'*'}, body='{"ip":"2606:4700:4700::1111"}'); return
                    if '.fixture123.bash.ws/' in url:
                        images.append(url); route.fulfill(status=204, body=''); return
                    route.abort('blockedbyclient')
                context.route('**/*', external)
                page = context.new_page()
                page.on('pageerror', lambda error: failures.append(str(error)))
                try:
                    page.goto(root)
                    page.click('#clash-region-start')
                    page.wait_for_function("document.getElementById('clash-region-status').textContent.includes('已回传')", timeout=30000)
                    assert len(reports) == 1 and not failures
                    report = reports[0]
                    assert report['kind'] == 'browser' and report['schema'] == 1
                    assert report['timezone'] == timezone and report['languages'][0] == locale
                    assert report['geolocation']['latitude'] is None
                    assert page.evaluate('window.clashLocationRequests') == 0, 'Detection requested location without a click'
                    if mode == 'complete':
                        assert report['ipv4'] == '8.8.8.8' and ':' in report['ipv6']
                        assert report['webrtc']['completed'] and 'fixture.local' in report['webrtc']['candidates']
                        assert page.evaluate('window.clashRtcClosed') == 1
                        assert report['dns_id'] == 'fixture123' and len(images) == 8
                        context.grant_permissions(['geolocation'])
                        page.click('#clash-region-location')
                        page.wait_for_function("document.getElementById('clash-region-status').textContent.includes('已回传') && window.clashLocationRequests === 1", timeout=30000)
                        # Wait for the fresh collection and second HTTP report.
                        page.wait_for_function("document.getElementById('clash-region-results').textContent.includes('37.7')", timeout=30000)
                        deadline = time.monotonic()+5
                        while len(reports) < 2 and time.monotonic() < deadline: time.sleep(.05)
                        assert len(reports) == 2 and reports[-1]['geolocation']['latitude'] == 37.7
                    elif mode == 'absent':
                        assert not report['webrtc']['supported'] and report['ipv6'] == ''
                    else:
                        assert not report['webrtc']['completed'] and report['webrtc']['error']
                        assert report['ipv4'] == report['ipv6'] == '' and report['dns_id'] == '' and report['dns_error']
                        assert page.evaluate('window.clashRtcClosed') == 1
                    page.screenshot(path=str(output/('browser-'+mode+'.png')), full_page=True)
                    cases.append({'case':mode, 'status':'passed', 'locale':locale, 'timezone':timezone,
                                  'location_only_on_click':True, 'external_services':'intercepted fixtures'})
                finally:
                    context.close(); server.shutdown(); server.server_close()
        finally: browser.close()
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--executable', type=Path)
    parser.add_argument('--browser', action='store_true')
    parser.add_argument('--output', type=Path, default=ROOT/'target/claude-report')
    args = parser.parse_args()
    if not args.executable and not args.browser: parser.error('Select --executable and/or --browser')
    args.output.mkdir(parents=True, exist_ok=True)
    result = {'schema':1, 'status':'running', 'cases':[]}
    started = time.monotonic()
    try:
        if args.executable: result['terminal'] = terminal(args.executable.resolve(strict=True))
        if args.browser: result['cases'] = browsers(args.output)
        result['status'] = 'passed'
    except BaseException as error:
        result.update(status='failed', error=str(error))
        raise
    finally:
        result['duration_seconds'] = round(time.monotonic()-started, 3)
        write_json(args.output/'report.json', result)


if __name__ == '__main__': main()
