"""Compile pinned detector decisions to Rust at development time.

No shell syntax, interpreter, or external HTTP tools are shipped or executed by
the application. Unsupported source constructs fail generation explicitly.
The generated code and its upstream-derived logic retain AGPL-3.0-only.
"""
import json
import re
from pathlib import Path
import argparse
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parent.parent


def lit(s):
    n = 1
    while '"' + '#' * n in s:
        n += 1
    return 'r' + '#' * n + '"' + s + '"' + '#' * n


def split(s, separator=None):
    """Split only outside quotes and command substitutions; retain quoting."""
    parts, start, i, quote, depth = [], 0, 0, None, 0
    while i < len(s):
        c = s[i]
        if c == '\\' and quote != "'":
            i += 2
            continue
        if quote == "'":
            if c == "'":
                quote = None
        elif s[i:i+2] == '$(':
            depth += 1
            i += 1
        elif depth:
            if c == ')':
                depth -= 1
            elif c == '(':
                depth += 1
        elif c == '"':
            quote = None if quote == '"' else '"'
        elif c == "'" and quote is None:
            quote = "'"
        elif quote is None and ((separator and c == separator) or (separator is None and c.isspace())):
            if s[start:i].strip():
                parts.append(s[start:i].strip())
            start = i + 1
        i += 1
    if s[start:].strip():
        parts.append(s[start:].strip())
    if depth or quote:
        raise ValueError('Unbalanced expression: ' + s)
    return parts


def word(s):
    pieces, chunk, i, quote = [], '', 0, None
    def flush():
        nonlocal chunk
        if chunk:
            pieces.append(lit(chunk) + '.to_owned()')
            chunk = ''
    while i < len(s):
        c = s[i]
        if c == "'" and quote != '"':
            quote = None if quote == "'" else "'"
            i += 1
            continue
        if c == '"' and quote != "'":
            quote = None if quote == '"' else '"'
            i += 1
            continue
        if c == '\\' and quote != "'" and i + 1 < len(s):
            next_char = s[i+1]
            if quote is None or next_char in '\\"$`':
                chunk += next_char
                i += 2
                continue
        if c == '$' and quote != "'":
            flush()
            if s[i:i+2] == '$(':
                j, depth, q = i + 2, 1, None
                while j < len(s):
                    if s[j] == '\\' and q != "'":
                        j += 2
                        continue
                    if s[j] in "'\"":
                        q = None if q == s[j] else s[j] if q is None else q
                    if q is None:
                        if s[j] == '(':
                            depth += 1
                        elif s[j] == ')':
                            depth -= 1
                            if not depth:
                                break
                    j += 1
                if depth:
                    raise ValueError('Unclosed substitution: ' + s)
                pieces.append(expression(s[i+2:j]))
                i = j + 1
                continue
            if s[i:i+2] == '${':
                j = s.index('}', i+2)
                name = s[i+2:j]
                m = re.fullmatch(r'(\w+)(\^\^|:.*|#.*)?', name)
                if not m:
                    raise ValueError('Unsupported expansion: ' + name)
                value = 'v.get(' + lit(m[1]) + ')'
                suffix = m[2] or ''
                if suffix == '^^':
                    value += '.to_uppercase()'
                elif suffix.startswith(':'):
                    if name == 'RANDOM:0-1':
                        value = 'random_digit()'
                    else:
                        bounds = suffix[1:].split(':')
                        a = int(bounds[0].strip())
                        b = 'None' if len(bounds) == 1 else 'Some(' + str(int(bounds[1])) + ')'
                        value = f'slice_text(&{value}, {a}, {b})'
                elif suffix.startswith('#'):
                    value = f'trim_prefix_pattern(&{value}, &{word(suffix[1:])})'
                pieces.append(value)
                i = j + 1
                continue
            m = re.match(r'\$(\w+|[?])', s[i:])
            if m:
                pieces.append('v.get(' + lit(m[1]) + ')')
                i += len(m[0])
                continue
        chunk += c
        i += 1
    flush()
    return 'String::new()' if not pieces else pieces[0] if len(pieces) == 1 else '[' + ', '.join(pieces) + '].concat()'


def plain(token):
    # Only for static option/selector strings, never evaluate shell expressions.
    if '$' in token:
        raise ValueError('Expected static token: ' + token)
    if token.startswith("'") and token.endswith("'"):
        return token[1:-1]
    if token.startswith('"') and token.endswith('"'):
        return token[1:-1].replace('\\"', '"').replace('\\\\', '\\')
    return re.sub(r'\\(.)', r'\1', token)


def vector(tokens):
    return '&[' + ', '.join(word(t) for t in tokens) + ']'


def http(tokens):
    url, headers, data = None, [], []
    method, head, include, follow, fail, discard, timeout, writeout = None, False, False, False, False, False, 10, None
    cookie_store, cookie_load, retries, tls13 = False, False, 0, False
    i = 0
    while i < len(tokens):
        t = tokens[i]
        if t in ['$curlArgs', '$usePROXY', '$xForward', '-${1}']:
            i += 1
            continue
        if t.startswith(('"http', "'http", 'http')) or t in ['"$target_url"', '"${req_url}"']:
            if url is not None:
                raise ValueError('Multiple request URLs')
            url = word(t)
        elif t in ['--user-agent', '-A']:
            i += 1
            headers.append('[' + lit('user-agent') + '.to_owned(), ' + word(tokens[i]) + ']')
        elif t in ['-H', '--header']:
            i += 1
            headers.append('header_pair(&' + word(tokens[i]) + ')?')
        elif t in ['-X', '--request']:
            i += 1
            method = plain(tokens[i])
        elif t in ['-d', '--data', '--data-raw', '--data-binary', '--data-urlencode']:
            i += 1
            data.append(word(tokens[i]))
        elif t in ['-b', '--cookie']:
            i += 1
            if tokens[i] == 'bahamut_cookie.txt':
                cookie_load = True
            else:
                headers.append('[' + lit('cookie') + '.to_owned(), ' + word(tokens[i]) + ']')
        elif t == '--cookie-jar':
            i += 1
            if tokens[i] != 'bahamut_cookie.txt':
                raise ValueError('Unknown cookie store')
            cookie_store = True
        elif t == '--retry':
            i += 1
            retries = int(tokens[i])
        elif t in ['-w', '--write-out']:
            i += 1
            writeout = word(tokens[i])
        elif t in ['-o', '--output']:
            i += 1
            if plain(tokens[i]) != '/dev/null':
                raise ValueError('Unsupported output file: ' + tokens[i])
            discard = True
        elif t in ['--max-time', '-m']:
            i += 1
            timeout = int(plain(tokens[i]))
        elif t == '--resolve':
            i += 1  # IPv6-only source branch; native execution uses the loopback proxy.
        elif t == '--tlsv1.3':
            tls13 = True
        elif re.fullmatch(r'-[sSfLiI46]+', t):
            head |= 'I' in t
            include |= 'i' in t
            follow |= 'L' in t
            fail |= 'f' in t
        else:
            raise ValueError('Unsupported HTTP option: ' + t)
        i += 1
    if url is None:
        raise ValueError('Missing request URL: ' + repr(tokens))
    method = method or ('HEAD' if head else 'POST' if data else 'GET')
    body = 'None' if not data else 'Some([' + ', '.join(data) + '].join("&"))'
    w = 'String::new()' if writeout is None else writeout
    return 'ctx.request(Request { url: ' + url + ', method: ' + lit(method) + '.to_owned(), headers: vec![' + ', '.join(headers) + '], body: ' + body + ', follow: ' + str(follow).lower() + ', headers_only: ' + str(head).lower() + ', include_headers: ' + str(include).lower() + ', fail_status: ' + str(fail).lower() + ', discard_body: ' + str(discard).lower() + ', timeout: ' + str(timeout) + ', writeout: ' + w + ', cookie_store: ' + str(cookie_store).lower() + ', cookie_load: ' + str(cookie_load).lower() + ', retries: ' + str(retries) + ', tls13: ' + str(tls13).lower() + ' }).await?'


def expression(s):
    # Redirection only suppressed diagnostics; there is no filesystem redirection
    # in the native code. Remove it before tokenizing, outside quoted strings.
    s = re.sub(r'\s+2>(?:&1|/dev/null)', '', s)
    s = re.sub(r'\s+>(?:/dev/null)', '', s)
    pipeline = split(s, '|')
    result = None
    for index, stage in enumerate(pipeline):
        tokens = split(stage)
        if not tokens:
            raise ValueError('Empty pipeline')
        op, args = tokens[0], tokens[1:]
        if index == 0:
            if op == 'curl':
                result = http(args)
            elif op in ['echo', 'printf']:
                if op == 'echo':
                    decode = '-e' in args
                    args = [t for t in args if t not in ['-e', '-n']]
                    result = '[' + ', '.join(word(t) for t in args) + '].join(" ")'
                    if decode:
                        result = 'decode_escapes(&' + result + ')'
                else:
                    if plain(args[0]) != '%s':
                        raise ValueError('Unsupported format')
                    result = word(args[1])
            elif op == 'cat' and args == ['/dev/urandom']:
                return 'random_id()'
            elif op == 'date':
                result = 'timestamp(' + str(plain(args[0]) == '+%s%3N').lower() + ')'
            elif op == 'getent':
                result = 'ctx.resolve_host(&' + word(args[1]) + ', &' + word(args[0]) + ').await?'
            elif op == 'detect_isp':
                result = 'ctx.detect_isp(&' + word(args[0]) + ').await?'
            else:
                raise ValueError('Unsupported source operation: ' + stage)
        elif op in ['jq', 'python']:
            selector = '"."' if op == 'python' else word(args[0])
            result = 'json_field(&' + result + ', &' + selector + ')?'
        elif op == 'grep':
            result = 'select_lines(&' + result + ', ' + vector(args) + ')?'
        elif op == 'awk':
            result = 'select_fields(&' + result + ', ' + vector(args) + ')?'
        elif op == 'sed':
            result = 'replace_text(&' + result + ', ' + vector(args) + ')?'
        elif op == 'cut':
            result = 'split_fields(&' + result + ', ' + vector(args) + ')?'
        elif op == 'tr':
            result = 'map_characters(&' + result + ', ' + vector(args) + ')?'
        elif op == 'head':
            result = 'take_lines(&' + result + ', ' + vector(args) + ')?'
        elif op == 'sort':
            result = 'sort_lines(&' + result + ')'
        elif op == 'uniq':
            result = 'unique_lines(&' + result + ')'
        elif op == 'xargs':
            result = '(' + result + ').split_whitespace().collect::<Vec<_>>().join(" ")'
        elif op == 'openssl' and args[:3] == ['dgst', '-sha1', '-hmac']:
            result = 'sign_sha1(&' + result + ', &' + word(args[3]) + ')?'
        elif op == 'openssl' and args == ['base64']:
            pass  # sign_sha1 already returns base64, without external processes.
        else:
            raise ValueError('Unsupported transformation: ' + stage)
    return result


def condition(s):
    s = re.sub(r';?\s*then\s*$', '', s).strip().rstrip(';').strip()
    tokens = split(s)
    groups, current = [], []
    for t in tokens:
        if t in ['&&', '||']:
            groups.extend([test(current), t])
            current = []
        else:
            current.append(t)
    groups.append(test(current))
    return ' '.join(groups)


def test(tokens):
    if tokens[0] not in ['[', '[[']:
        return '!(' + expression(' '.join(tokens)) + ').is_empty()'
    glob = tokens[0] == '[['
    tokens = tokens[1:-1]
    negated = tokens[0] == '!'
    if negated:
        tokens = tokens[1:]
    if tokens[0] in ['-n', '-z']:
        result = '(' + word(tokens[1]) + ').is_empty()'
        if tokens[0] == '-n':
            result = '!' + result
    elif len(tokens) == 3:
        a, op, b = tokens
        if op in ['-eq', '-ne', '-gt', '-lt', '-ge', '-le']:
            op = {'-eq':'==','-ne':'!=','-gt':'>','-lt':'<','-ge':'>=','-le':'<='}[op]
            result = 'number(&' + word(a) + ') ' + op + ' number(&' + word(b) + ')'
        elif op in ['==', '=', '!=']:
            if glob and ('*' in b or '?' in b) and not (b.startswith('"') and b.endswith('"')):
                result = 'matches_pattern(&' + word(a) + ', &' + word(b) + ')'
                if op == '!=':
                    result = '!' + result
            else:
                result = word(a) + (' != ' if op == '!=' else ' == ') + word(b)
        else:
            raise ValueError('Unsupported comparison: ' + op)
    else:
        raise ValueError('Unsupported test: ' + repr(tokens))
    return '(!(' + result + '))' if negated else '(' + result + ')'


def compile_function(name, body):
    # Correct three pre-existing upstream spelling mistakes while retaining its
    # decisions. Previously these comparisons used unset variables.
    if name == 'MediaUnlockTest_7plus':
        body = body.replace('$GetPlayURL', '$result1')
    if name == 'MediaUnlockTest_EroGameSpace':
        body = body.replace('$countrycode', '$result')
    body = body.replace('${UA_BROWSER}', '${UA_Browser}')
    output = ['async fn ' + re.sub(r'\W', '_', name).lower() + '(ctx: &mut Context) -> Result<String> {', '    let mut v = Variables::new();', '    let mut output = String::new();']
    if name in ['MediaUnlockTest_NBCTV', 'MediaUnlockTest_TLCGO']:
        output.append('v.set("onetrustresult", ctx.request(Request::get("https://geolocation.onetrust.com/cookieconsentpub/v1/geo/location/dnsfeed")).await?);')
    in_case = False
    for original in body.splitlines():
        line = original.strip()
        if not line or line.startswith('#'):
            continue
        if line.startswith('if '):
            output.append('if ' + condition(line[3:]) + ' {')
        elif line.startswith('elif '):
            output.append('} else if ' + condition(line[5:]) + ' {')
        elif line == 'else':
            output.append('} else {')
        elif line == 'fi':
            output.append('}')
        elif line.startswith('case '):
            in_case = True
            output.append('match ' + word(split(line)[1]) + '.as_str() {')
        elif line == 'esac':
            output.append('}')
            in_case = False
        elif in_case:
            arm, command = line.split(')', 1)
            value = '_' if arm.strip() == '*' else lit(plain(arm.strip()))
            output.append(value + ' => { output = ' + expression(command.strip().removesuffix(';;').strip()) + '; },')
        elif line.startswith('return'):
            output.append('return Ok(output);')
        elif line == 'rm -f bahamut_cookie.txt':
            output.append('ctx.clear_cookies();')
        elif line.startswith('echo ') or line.startswith('printf '):
            if '|' in line:
                output.append('v.set("?", if (' + expression(line) + ').is_empty() { "1" } else { "0" }.to_owned());')
            else:
                output.append('output = ' + expression(line) + ';')
        else:
            line = line.removeprefix('local ')
            m = re.fullmatch(r'(\w+)=(.*)', line)
            if not m:
                raise ValueError('Unsupported statement: ' + original)
            output.append('v.set(' + lit(m[1]) + ', ' + word(m[2]) + ');')
    output.extend(['Ok(output)', '}'])
    return '\n'.join(output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='Verify committed output without modifying it')
    args = parser.parse_args()
    source = (ROOT/'vendor/region-restriction-check/check-adapted.sh').read_text(encoding='utf-8')
    catalog = json.loads((ROOT/'resources/ip-check/services.json').read_text(encoding='utf-8'))
    ids = [s['id'] for s in catalog if s['id'] not in ['exit-ip', 'github']]
    functions = {}
    for m in re.finditer(r'^(?:function\s+)?([^\s(]+)\s*\(\)\s*\{', source, re.M):
        end = re.search(r'^}\s*$', source[m.end():], re.M)
        functions[m[1]] = source[m.end():m.end()+end.start()]
    result, errors = [], []
    for name in ids:
        try:
            result.append(compile_function(name, functions[name]))
        except Exception as e:
            errors.append((name,str(e)))
    if errors:
        for name, error in errors:
            print(name + ': ' + error)
        raise SystemExit(f'{len(errors)} functions failed; generated output not changed')
    dispatch = '\n'.join(lit(name) + ' => ' + re.sub(r'\W','_',name).lower() + '(ctx).await,' for name in ids)
    header = '// SPDX-License-Identifier: AGPL-3.0-only\n// Generated by scripts/native-detector-codegen.py from pinned upstream decisions.\n// No shell interpreter or external utility is used at runtime.\nuse super::*;\n\n'
    header += 'pub(super) async fn execute(id: &str, ctx: &mut Context) -> Result<String> { match id {\n' + dispatch + '\n_ => bail!("检测项目不存在"),\n} }\n\n'
    destination = ROOT/'src/region_check/generated.rs'
    destination.parent.mkdir(exist_ok=True)
    generated = header + '\n\n'.join(result) + '\n'
    with tempfile.TemporaryDirectory(prefix='detector-codegen-') as folder:
        temporary = Path(folder)/'generated.rs'
        temporary.write_text(generated, encoding='utf-8', newline='\n')
        subprocess.run(['rustfmt','--edition','2024',str(temporary)],check=True)
        formatted = temporary.read_text(encoding='utf-8')
    if args.check:
        if destination.read_text(encoding='utf-8') != formatted:
            raise SystemExit('Generated Rust differs; run scripts/native-detector-codegen.py')
    else:
        destination.write_text(formatted,encoding='utf-8',newline='\n')
    print(f'Compiled all {len(ids)} platform functions to native Rust')


if __name__ == '__main__':
    main()
