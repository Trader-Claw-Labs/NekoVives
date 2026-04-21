#!/usr/bin/env bash
set -euo pipefail

REPO="Trader-Claw-Labs/Trader-Claw"
BIN_NAME="trader-claw"
INSTALL_DIR="/usr/local/bin"

print_info()  { echo "  \e[34m\u2139\e[0m  $*"; }
print_ok()    { echo "  \e[32m\u2713\e[0m  $*"; }
print_err()   { echo "  \e[31m\u2717\e[0m  $*" >&2; }

# Detect OS
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
case "$OS" in
  linux*)  OS="linux" ;;
  darwin*) OS="macos" ;;
  *)
    print_err "Unsupported OS: $OS"
    print_err "Please download manually from https://github.com/$REPO/releases"
    exit 1
    ;;
esac

# Detect architecture
ARCH=$(uname -m)
case "$ARCH" in
  x86_64)          ARCH="x86_64" ;;
  aarch64 | arm64) ARCH="arm64" ;;
  *)
    print_err "Unsupported architecture: $ARCH"
    exit 1
    ;;
esac

# Check for required tools
for tool in curl tar; do
  if ! command -v "$tool" &>/dev/null; then
    print_err "Required tool not found: $tool"
    exit 1
  fi
done

# Fetch latest version tag
print_info "Fetching latest release..."
VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
  | grep '"tag_name"' \
  | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')

if [[ -z "$VERSION" ]]; then
  print_err "Could not determine latest version. Check your internet connection."
  exit 1
fi

print_info "Latest version: $VERSION"

# Check if already installed and up-to-date
if command -v "$BIN_NAME" &>/dev/null; then
  CURRENT=$("$BIN_NAME" --version 2>/dev/null | awk '{print $2}' || echo "unknown")
  if [[ "v$CURRENT" == "$VERSION" ]]; then
    print_ok "$BIN_NAME $VERSION is already installed and up-to-date."
    exit 0
  fi
  print_info "Updating from $CURRENT to $VERSION..."
fi

# Build download URL
ARTIFACT="${BIN_NAME}-${OS}-${ARCH}.tar.gz"
URL="https://github.com/$REPO/releases/download/$VERSION/$ARTIFACT"

print_info "Downloading $ARTIFACT..."
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

curl -fsSL "$URL" -o "$TMP_DIR/$ARTIFACT"

# Verify checksum if checksums.txt is available
CHECKSUM_URL="https://github.com/$REPO/releases/download/$VERSION/checksums.txt"
if curl -fsSL "$CHECKSUM_URL" -o "$TMP_DIR/checksums.txt" 2>/dev/null; then
  print_info "Verifying checksum..."
  cd "$TMP_DIR"
  grep "$ARTIFACT" checksums.txt | sha256sum --check --quiet
  cd - > /dev/null
  print_ok "Checksum verified."
fi

# Extract
tar -xzf "$TMP_DIR/$ARTIFACT" -C "$TMP_DIR"

# Install
if [[ -w "$INSTALL_DIR" ]]; then
  mv "$TMP_DIR/$BIN_NAME" "$INSTALL_DIR/$BIN_NAME"
else
  print_info "Requesting sudo to install to $INSTALL_DIR..."
  sudo mv "$TMP_DIR/$BIN_NAME" "$INSTALL_DIR/$BIN_NAME"
fi
chmod +x "$INSTALL_DIR/$BIN_NAME"

print_ok "Trader Claw $VERSION installed to $INSTALL_DIR/$BIN_NAME"
print_ok "Run: trader-claw gateway"
