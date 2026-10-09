"""Generate the exact synthetic subscription used for the GUI comparison.

Usage: python3 gui-fixture.py /tmp/memory-workload.yaml
Import and activate the output in an isolated test data directory. Set proxy mode
Off and scheduled delay checks to 0. No node here connects to an external server.
"""
import sys
from pathlib import Path

output = Path(sys.argv[1])
with output.open("x", encoding="utf-8") as stream:
    stream.write("proxies:\n")
    for index in range(2000):
        stream.write(f"  - {{name: node-{index}, type: http, server: 127.0.0.1, port: 1}}\n")
    stream.write("proxy-groups:\n")
    members = ", ".join(f"node-{index}" for index in range(2000))
    for group in range(40):
        stream.write(f"  - name: group-{group}\n    type: select\n    proxies: [{members}]\n")
    stream.write("rules:\n")
    for index in range(20000):
        stream.write(f"  - DOMAIN-SUFFIX,example-{index}.test,group-0\n")
    stream.write("  - MATCH,DIRECT\n")
