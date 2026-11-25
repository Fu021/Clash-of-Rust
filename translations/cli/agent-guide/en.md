# Clash of Rust · Agent Guide

Clash of Rust is a proxy client powered by Mihomo. On Windows, macOS, and Linux, the desktop app, terminal UI, and `cor` share subscriptions, settings, and a backend. Run commands as the account that uses the desktop app. If the user runs a separate instance, use its `--data-dir PATH` consistently.

## Start with the installed version

Run `cor --version`, `cor help all`, and `cor help <command>` to confirm what the installed version supports. `cor` and `cor ui` open the terminal interface in an interactive terminal. For automation, call the specific subcommand and use `--json` for a single result or `--jsonl` for a stream. Check both the exit code and the relevant result fields.

## Diagnose before changing settings

```sh
cor status --json
cor proxy --json
cor profile list --json
cor diagnose --offline --json
```

These commands inspect the current instance without starting the core. Check whether it is running, which subscription is active, and whether System Proxy or TUN is enabled. When an active network probe is appropriate, `cor network check https://example.com --json` reports DNS results and the route observed for that request. To save local diagnostics, run `cor diagnose --offline --output cor-diagnose.json` with a fresh output path.

## Make only the requested changes

Use the CLI to change subscriptions, rules, and settings. Identify subscriptions by ID when names are ambiguous. If Claude Protection blocks an operation, report that state to the user. Confirm the intended proxy mode before changing System Proxy or TUN, and let the user handle any operating-system authorization prompt. Some commands that need network access may start the core and restore its saved proxy mode. After a timeout or interruption during a change, check the current state before retrying.

Keep the default credential redaction. Review reports before sharing them, including URLs, domains, IP addresses, and local identifiers. Keep subscription URLs and `control.json` private.

## Project links

- [GitHub repository](https://github.com/Fu021/Clash-of-Rust)
- [Documentation index](https://github.com/Fu021/Clash-of-Rust/blob/main/docs/README.md)
- [CLI reference](https://github.com/Fu021/Clash-of-Rust/blob/main/docs/cli.md)
- [Issue forms](https://github.com/Fu021/Clash-of-Rust/issues/new/choose)
