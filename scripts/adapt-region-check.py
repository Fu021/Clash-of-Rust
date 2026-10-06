"""Generate an AGPL adapter from the pinned upstream, without startup/install code."""
import json
import re
from pathlib import Path

root = Path(__file__).resolve().parent.parent
vendor = root / "vendor/region-restriction-check"
source = (vendor / "upstream/check.sh").read_text(encoding="utf-8")
functions = {}
for match in re.finditer(r"^(?:function\s+)?([^\s(]+)\s*\(\)\s*\{", source, re.M):
    end = re.search(r"^}\s*$", source[match.end():], re.M)
    if not end:
        raise ValueError(f"Unclosed function {match[1]}")
    functions[match[1]] = source[match.start():match.end() + end.end()]
services = json.loads((root / "resources/ip-check/services.json").read_text(encoding="utf-8"))
ids = [item["id"] for item in services if item["id"] not in ("exit-ip", "github")]
assert len(ids) == 181 and all(item in functions for item in ids)
header = '''#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
# Adapted from 1-stream/RegionRestrictionCheck; see SOURCE.md for revision/changes.
COR_SERVICE="$1"
COR_PROXY="$2"
COR_TIMINGS="$3"
export COR_TIMINGS
COR_DIR="$(cd -- "$(dirname -- "$0")" && pwd)"
export PATH="$COR_DIR/runtime/usr/bin:$PATH"
export LC_ALL=C
export CURL_CA_BUNDLE="$COR_DIR/runtime/ca-bundle.crt"
if command -v cygpath >/dev/null 2>&1; then
    export TMPDIR="$(cygpath -u "$COR_WORK")"
else
    export TMPDIR="$COR_WORK"
fi
unset BASH_ENV ENV http_proxy https_proxy all_proxy HTTP_PROXY HTTPS_PROXY ALL_PROXY NO_PROXY no_proxy
Media_Cookie=$(<"$COR_DIR/cookies")
IATACode=$(<"$COR_DIR/IATACode.txt")
UA_Browser="Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/112.0.0.0 Safari/537.36 Edg/112.0.1722.64"
UA_Dalvik="Dalvik/2.1.0 (Linux; U; Android 9; ALP-AL00 Build/HUAWEIALP-AL00)"
curlArgs="--max-time 10"
Font_Black="\\033[30m"; Font_Red="\\033[31m"; Font_Green="\\033[32m"
Font_Yellow="\\033[33m"; Font_Blue="\\033[34m"; Font_Purple="\\033[35m"
Font_SkyBlue="\\033[36m"; Font_White="\\033[37m"; Font_Suffix="\\033[0m"
# Upstream only uses Python for JSON pretty printing. jq avoids a Python runtime.
python() { [[ "$*" == "-m json.tool" ]] || return 127; command jq --ascii-output --indent 4 .; }
getent() { "$COR_HELPER" --ip-check-resolve "$@"; }
# Force every request through mihomo, including secondary authentication endpoints.
# curl 8.3+ writes timings to a separate file without changing upstream response text.
curl() {
    local args=() writeout="" limit=0 headers status
    headers=$(mktemp "$TMPDIR/headers.XXXXXX") || return 1
    while (( $# )); do
        case "$1" in
            -w|--write-out) writeout="$2"; shift 2 ;;
            --write-out=*) writeout="${1#*=}"; shift ;;
            -w?*) writeout="${1#-w}"; shift ;;
            --max-time|-m) limit=1; args+=("$1" "$2"); shift 2 ;;
            --max-time=*|-m?*) limit=1; args+=("$1"); shift ;;
            *) args+=("$1"); shift ;;
        esac
    done
    (( limit )) || args+=(--max-time 12)
    command curl "${args[@]}" --proxy "$COR_PROXY" --noproxy "" --connect-timeout 5 --dump-header "$headers" --write-out "$writeout%output{>>$COR_TIMINGS}%{time_starttransfer}\\n%output{>>$COR_WORK/statuses.txt}%{http_code}\\n"
    status=$?
    # Browser challenges are not evidence of geographic restrictions.
    if grep -qi '^cf-mitigated: *challenge' "$headers"; then
        printf 'challenge\\n' >> "$COR_WORK/challenges.txt"
    fi
    if (( status )); then printf 'transport\\n' >> "$COR_WORK/failures.txt"; fi
    rm -f -- "$headers"
    return "$status"
}
'''
footer = '\ncase "$COR_SERVICE" in\n' + '|'.join(ids) + ') ;;\n*) exit 64 ;;\nesac\n'
footer += '''
# NBC's original function shares the OneTrust result from TLC; supply it independently.
if [[ "$COR_SERVICE" == "MediaUnlockTest_NBCTV" ]]; then
    onetrustresult=$(curl -sS "https://geolocation.onetrust.com/cookieconsentpub/v1/geo/location/dnsfeed" 2>&1)
fi
"$COR_SERVICE" 4
wait
'''
selected = [functions["detect_isp"]] + [functions[item] for item in ids]
# HTTP field names are case-insensitive. Windows curl uses HTTP/1.1, whose
# Location header is commonly capitalized; do not lowercase JSON/body values.
selected = [item.replace("grep 'location'", "grep -i '^location:'")
            .replace("grep -E 'x-crackle-region:|curl'", "grep -Ei 'x-crackle-region:|curl'")
            for item in selected]
# ChatGPT can now serve its landing page with 200 instead of a redirect.
# This establishes web reachability, not authenticated access or API unlock.
old = '''    \techo -n -e "\\r ChatGPT:\\t\\t\\t\\t${Font_Red}No${Font_Suffix}\\n"'''
new = '''        local main_status=$(echo "$tmpresult" | awk '/^HTTP\\// { code=$2 } END { print code }')
        local trace=$(curl $curlArgs -${1} --user-agent "${UA_Browser}" -SsL --max-time 10 "https://chatgpt.com/cdn-cgi/trace" 2>&1)
        local region=""
        if echo "$trace" | grep -q '^h=chatgpt.com'; then
            region=$(echo "$trace" | grep -E '^loc=[A-Z]{2}' | cut -d= -f2 | tr -d '\\r')
        fi
        if [[ "$main_status" == "200" ]]; then
            echo -n -e "\\r ChatGPT:\\t\\t\\t\\t${Font_Green}Web Reachable (Region: ${region})${Font_Suffix}\\n"
        else
            echo -n -e "\\r ChatGPT:\\t\\t\\t\\t${Font_Yellow}No (Region: ${region})${Font_Suffix}\\n"
        fi'''
chatgpt = next(index for index, item in enumerate(selected) if item.startswith("function MediaUnlockTest_ChatGPT()"))
assert old in selected[chatgpt], "Pinned ChatGPT function changed"
selected[chatgpt] = selected[chatgpt].replace(old, new, 1)
(vendor / "check-adapted.sh").write_text(header + '\n\n'.join(selected) + footer, encoding="utf-8", newline="\n")
print(f"Adapted {len(ids)} upstream functions; installer/menu/global startup excluded")
