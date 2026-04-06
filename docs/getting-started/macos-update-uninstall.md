# macOS Update and Uninstall Guide

This page documents supported update and uninstall procedures for TraderClaw on macOS (OS X).

Last verified: **February 22, 2026**.

## 1) Check current install method

```bash
which traderclaw
traderclaw --version
```

Typical locations:

- Homebrew: `/opt/homebrew/bin/traderclaw` (Apple Silicon) or `/usr/local/bin/traderclaw` (Intel)
- Cargo/bootstrap/manual: `~/.cargo/bin/traderclaw`

If both exist, your shell `PATH` order decides which one runs.

## 2) Update on macOS

### A) Homebrew install

```bash
brew update
brew upgrade traderclaw
traderclaw --version
```

### B) Clone + bootstrap install

From your local repository checkout:

```bash
git pull --ff-only
./bootstrap.sh --prefer-prebuilt
traderclaw --version
```

If you want source-only update:

```bash
git pull --ff-only
cargo install --path . --force --locked
traderclaw --version
```

### C) Manual prebuilt binary install

Re-run your download/install flow with the latest release asset, then verify:

```bash
traderclaw --version
```

## 3) Uninstall on macOS

### A) Stop and remove background service first

This prevents the daemon from continuing to run after binary removal.

```bash
traderclaw service stop || true
traderclaw service uninstall || true
```

Service artifacts removed by `service uninstall`:

- `~/Library/LaunchAgents/com.traderclaw.daemon.plist`

### B) Remove the binary by install method

Homebrew:

```bash
brew uninstall traderclaw
```

Cargo/bootstrap/manual (`~/.cargo/bin/traderclaw`):

```bash
cargo uninstall traderclaw || true
rm -f ~/.cargo/bin/traderclaw
```

### C) Optional: remove local runtime data

Only run this if you want a full cleanup of config, auth profiles, logs, and workspace state.

```bash
rm -rf ~/.traderclaw
```

## 4) Verify uninstall completed

```bash
command -v traderclaw || echo "traderclaw binary not found"
pgrep -fl traderclaw || echo "No running traderclaw process"
```

If `pgrep` still finds a process, stop it manually and re-check:

```bash
pkill -f traderclaw
```

## Related docs

- [One-Click Bootstrap](../one-click-bootstrap.md)
- [Commands Reference](../commands-reference.md)
- [Troubleshooting](../troubleshooting.md)
