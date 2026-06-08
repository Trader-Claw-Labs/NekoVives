#!/usr/bin/env bash
# ============================================================================
# NekoVives — AWS bootstrap (Ubuntu 22.04/24.04, us-east-1 recommended)
# ============================================================================
# Run this on a FRESH EC2 instance to install deps, build, and run the daemon.
# Goal #1: run NekoVives 24/7 independent of your home internet.
# Goal #2: measure real RTT to Polymarket's CLOB from us-east-1 (HFT viability).
#
# Recommended instance for the BUILD: t3.large (2 vCPU, 8 GB) — the workspace is
# large and a 4 GB box can OOM at link time. After it builds you can run the
# daemon on a smaller box (t3.small) by copying ./target/release/trader-claw.
#
# Usage on the instance:
#   chmod +x aws_bootstrap.sh && ./aws_bootstrap.sh
# ============================================================================
set -euo pipefail

REPO_URL="${REPO_URL:-https://github.com/Trader-Claw-Labs/NekoVives.git}"   # override if needed
WORKDIR="${WORKDIR:-$HOME/Trader-Claw}"

echo "==> [1/6] System deps"
sudo apt-get update -y
sudo apt-get install -y build-essential pkg-config libssl-dev git curl jq unzip ca-certificates

echo "==> [2/6] Rust toolchain"
if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  source "$HOME/.cargo/env"
fi
source "$HOME/.cargo/env" 2>/dev/null || true

echo "==> [3/6] Node.js 20 (for the web dashboard build)"
if ! command -v node >/dev/null 2>&1; then
  curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
  sudo apt-get install -y nodejs
fi

echo "==> [4/6] Clone / update repo"
if [ ! -d "$WORKDIR/.git" ]; then
  git clone "$REPO_URL" "$WORKDIR"
fi
cd "$WORKDIR"

echo "==> [5/6] Build web + binary (this takes 10-20 min on first run)"
( cd web && npm ci && npm run build )
cargo build --release

echo "==> [6/6] Latency probe to Polymarket CLOB (the HFT viability number)"
"$WORKDIR/scripts/deploy/latency_probe.sh" || true

cat <<EOF

============================================================================
DONE. NekoVives built at: $WORKDIR/target/release/trader-claw

Run the daemon (keeps running across SSH disconnects via nohup):
  cd $WORKDIR
  nohup ./target/release/trader-claw daemon > daemon.log 2>&1 &
  grep -m1 'X-Pairing-Code' daemon.log     # the one-time pairing code

Reach the dashboard from your laptop (SSH tunnel — gateway listens on 42617):
  ssh -i <key.pem> -L 42617:localhost:42617 ubuntu@<INSTANCE_PUBLIC_IP>
  # then open http://localhost:42617 in your browser, enter the pairing code

Keep it alive after you log out: the nohup above survives SSH; for a reboot-proof
setup, wrap it in a systemd unit (ask Claude for the unit file).
============================================================================
EOF
