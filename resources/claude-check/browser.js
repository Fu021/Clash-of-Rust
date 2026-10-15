/* Local diagnostic page and optional Claude-origin console sampler. No account access. */
async function collectClaudeRegion(options = {}) {
    const deadlineFetch = async (url, init = {}) => {
        const controller = new AbortController();
        const timer = setTimeout(() => controller.abort(), 5500);
        try {
            const response = await fetch(url, { ...init, signal: controller.signal, credentials: 'omit', cache: 'no-store', redirect: 'error' });
            const reader = response.body && response.body.getReader();
            const decoder = new TextDecoder();
            let text = '';
            if (reader) {
                try {
                    for (;;) {
                        const { value, done } = await reader.read();
                        if (done) break;
                        text += decoder.decode(value, { stream: true });
                        if (text.length > 65536) throw new Error('Response too large');
                    }
                    text += decoder.decode();
                } catch (error) { await reader.cancel(); throw error; }
                finally { reader.releaseLock(); }
            }
            return { ok: response.ok, json: async () => JSON.parse(text), text: async () => text };
        }
        finally { clearTimeout(timer); }
    };
    const exit = async (url) => {
        try { const response = await deadlineFetch(url); if (!response.ok) return ''; const data = await response.json(); return typeof data.ip === 'string' ? data.ip : ''; }
        catch (_) { return ''; }
    };
    const canvas = document.createElement('canvas');
    canvas.width = 160; canvas.height = 48;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    const fonts = [];
    const emoji = [];
    if (context) {
        const families = ['Microsoft YaHei', 'SimSun', 'SimHei', 'PingFang SC', 'PingFang TC', 'Noto Sans CJK SC', 'Source Han Sans SC', 'WenQuanYi Micro Hei'];
        const sample = 'AaWw0123中文漢字';
        for (const family of families) {
            const found = ['monospace', 'serif', 'sans-serif'].some(base => {
                context.font = `20px ${base}`;
                const width = context.measureText(sample).width;
                context.font = `20px "${family}", ${base}`;
                return Math.abs(context.measureText(sample).width - width) > 0.05;
            });
            if (found) fonts.push(family);
        }
        for (const [name, glyph] of [['中国旗', '\u{1F1E8}\u{1F1F3}'], ['美国旗', '\u{1F1FA}\u{1F1F8}'], ['日本旗', '\u{1F1EF}\u{1F1F5}']]) {
            context.clearRect(0, 0, 160, 48); context.fillStyle = '#111'; context.font = '30px sans-serif'; context.fillText(glyph, 4, 34);
            const pixels = context.getImageData(0, 0, 100, 48).data;
            let colored = false, ink = 0;
            for (let i = 0; i < pixels.length; i += 4) { if (pixels[i + 3] > 32) { ink++; if (Math.max(pixels[i], pixels[i + 1], pixels[i + 2]) - Math.min(pixels[i], pixels[i + 1], pixels[i + 2]) > 30) colored = true; } }
            emoji.push(`${name}：${colored ? '彩色字形' : ink ? '单色字形或字符回退' : '未获得字形'}`);
        }
    } else { emoji.push('Canvas 不可用'); }
    let canvas_hash = '', webgl_vendor = '', webgl_renderer = '';
    try {
        if (context && crypto.subtle) {
            const pixels = context.getImageData(0, 0, 160, 48).data;
            canvas_hash = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', pixels)), b => b.toString(16).padStart(2, '0')).join('');
        }
        const gl = document.createElement('canvas').getContext('webgl');
        if (gl) {
            const debug = gl.getExtension('WEBGL_debug_renderer_info');
            webgl_vendor = String(gl.getParameter(debug ? debug.UNMASKED_VENDOR_WEBGL : gl.VENDOR)).slice(0, 256);
            webgl_renderer = String(gl.getParameter(debug ? debug.UNMASKED_RENDERER_WEBGL : gl.RENDERER)).slice(0, 256);
            const lose = gl.getExtension('WEBGL_lose_context'); if (lose) lose.loseContext();
        }
    } catch (_) {}
    const rtc = async () => {
        const servers = ['stun:stun.cloudflare.com:3478', 'stun:stun.l.google.com:19302'];
        const result = { supported: typeof RTCPeerConnection !== 'undefined', completed: false, candidates: [], samples: [], servers, error: '' };
        if (!result.supported) { result.error = '浏览器未提供 WebRTC'; return result; }
        const probe = async server => {
        let peer;
        let timer;
        let completed = false;
        try {
            peer = new RTCPeerConnection({ iceServers: [{ urls: server }] });
            peer.createDataChannel('region-diagnostic');
            const gathered = new Promise(resolve => {
                timer = setTimeout(() => { result.error = 'ICE 收集超时'; resolve(); }, 4500);
                peer.onicecandidate = event => {
                    if (!event.candidate) { completed = true; resolve(); return; }
                    const candidate = event.candidate;
                    const address = candidate.address || candidate.candidate.split(/\s+/)[4] || '';
                    if (address && result.candidates.length < 24 && !result.candidates.includes(address)) result.candidates.push(address);
                    if (address && result.samples.length < 24) result.samples.push({ address, server, protocol: candidate.protocol || '', type: candidate.type || '' });
                };
            });
            await peer.setLocalDescription(await peer.createOffer());
            await gathered;
        } catch (_) { result.error = 'WebRTC 被阻止或初始化失败'; }
        finally { clearTimeout(timer); if (peer) peer.close(); }
        return completed;
        };
        result.completed = (await Promise.all(servers.map(probe))).every(Boolean);
        return result;
    };
    const dns = async () => {
        if (!options.root) return { id: '', error: 'Claude 页面采样未单独执行 DNS 测试；本地页面 DNS 不代替此站点结果。' };
        const images = [];
        try {
            const response = await deadlineFetch(options.root + 'dns/start', { method: 'POST', headers: { 'X-Clash-Diagnostic': '1' } });
            if (!response.ok) throw new Error();
            const data = await response.json();
            if (!/^[a-zA-Z0-9-]{1,64}$/.test(data.id)) throw new Error();
            for (let i = 0; i < 16; i++) { const image = new Image(); image.crossOrigin = 'anonymous'; image.referrerPolicy = 'no-referrer'; image.src = `https://${i}.${data.id}.bash.ws/diagnostic.png`; images.push(image); }
            await new Promise(resolve => setTimeout(resolve, 3500));
            return { id: data.id, error: '' };
        } catch (_) { return { id: '', error: 'DNS 测试服务不可用、被浏览器阻止或本地入口已关闭。' }; }
        finally { for (const image of images) { image.onload = null; image.onerror = null; image.src = ''; } }
    };
    let permission = '未确认';
    try { permission = (await navigator.permissions.query({ name: 'geolocation' })).state; } catch (_) {}
    const cf = async () => { try { const response = await deadlineFetch('https://www.cloudflare.com/cdn-cgi/trace'); return response.ok ? (await response.text()).slice(0, 2048) : ''; } catch (_) { return ''; } };
    const [ipv4, ipv6, webrtc, dnsResult, cf_trace] = await Promise.all([exit('https://api.ipify.org?format=json'), exit('https://api6.ipify.org?format=json'), rtc(), dns(), cf()]);
    let trace = '';
    if (options.site && location.protocol === 'https:' && /(^|\.)(claude\.ai|claude\.com|claude\.app|claudeusercontent\.com|claudemcpcontent\.com|anthropic\.com)$/.test(location.hostname)) {
        try { const response = await deadlineFetch(location.origin + '/cdn-cgi/trace'); if (response.ok) trace = (await response.text()).slice(0, 2048); } catch (_) {}
    }
    const intl = Intl.DateTimeFormat().resolvedOptions();
    const format = new Intl.DateTimeFormat(undefined, { dateStyle: 'short' }).format(new Date(2026, 0, 23));
    return { kind: 'browser', schema: 1, origin: location.origin, timezone: intl.timeZone || '', offset: new Date().getTimezoneOffset(), languages: Array.from(navigator.languages || [navigator.language]).filter(Boolean).slice(0, 24), locale: intl.locale || '', date_format: format, platform: navigator.platform || '', user_agent: navigator.userAgent || '', fonts, emoji, canvas_hash, webgl_vendor, webgl_renderer, cookie_enabled: typeof navigator.cookieEnabled === 'boolean' ? navigator.cookieEnabled : null, webrtc, ipv4, ipv6, dns_id: dnsResult.id, dns_error: dnsResult.error, trace, cf_trace, geolocation: { permission, latitude: null, longitude: null } };
}

// The console sampler executes the collector only, without manipulating the
// Claude page. DOM controls exist exclusively in our local diagnostic page.
if (document.getElementById('clash-region-start')) {
    const start = document.getElementById('clash-region-start');
    const locate = document.getElementById('clash-region-location');
    const download = document.getElementById('clash-region-download');
    const status = document.getElementById('clash-region-status');
    const result = document.getElementById('clash-region-results');
    const progress = document.getElementById('clash-region-progress');
    const root = new URL('.', location.href).href;
    let data;
    const submit = async () => {
        result.textContent = JSON.stringify(data, null, 2);
        download.disabled = false;
        status.textContent = '正在回传并查询地区信息…';
        const response = await fetch(root + 'report', { method: 'POST', credentials: 'omit', headers: { 'X-Clash-Diagnostic': '1', 'Content-Type': 'application/json' }, body: JSON.stringify(data) });
        if (!response.ok) throw new Error();
        status.textContent = '检测报告已回传到 Clash of Rust。你可以关闭此页面。'; progress.value = 4;
    };
    const run = async () => {
        if (start.disabled) return;
        start.disabled = true; locate.disabled = true; status.textContent = '采集浏览器与网络地区线索…'; progress.value = 1;
        try { data = await collectClaudeRegion({ root }); progress.value = 3; await submit(); }
        catch (_) { status.textContent = '回传失败；请确认应用中的检测入口仍开启。可以保存采样 JSON 后复制内容导入。'; }
        finally { start.disabled = false; locate.disabled = false; }
    };
    start.onclick = run;
    locate.onclick = async () => {
        if (!navigator.geolocation) { status.textContent = '浏览器未提供定位功能。'; return; }
        if (!data) { status.textContent = '请先完成浏览器检测，再主动检测定位。'; return; }
        locate.disabled = true;
        navigator.geolocation.getCurrentPosition(async position => {
            try {
                data = await collectClaudeRegion({ root });
                data.geolocation = { permission: 'granted', latitude: position.coords.latitude, longitude: position.coords.longitude };
                await submit();
            } catch (_) { status.textContent = '定位回传失败，可重新检测并导入 JSON。'; }
            finally { locate.disabled = false; }
        }, () => { status.textContent = '定位未获授权或没有返回结果。'; locate.disabled = false; }, { timeout: 8000, maximumAge: 0 });
    };
    download.onclick = () => {
        if (!data) return;
        const blob = new Blob([JSON.stringify(data, null, 2)], { type: 'application/json' });
        const url = URL.createObjectURL(blob); const a = document.createElement('a'); a.href = url; a.download = 'claude-browser-sample.json'; a.click(); setTimeout(() => URL.revokeObjectURL(url), 1000);
    };
    void run();
}
