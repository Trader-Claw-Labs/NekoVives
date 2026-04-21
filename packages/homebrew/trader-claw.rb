# Trader Claw Homebrew Formula
# Maintained at: https://github.com/Trader-Claw-Labs/Trader-Claw
#
# To use:
#   brew tap Trader-Claw-Labs/trader-claw https://github.com/Trader-Claw-Labs/homebrew-trader-claw
#   brew install trader-claw
#
# SHA256 placeholders below are updated automatically by .github/workflows/update-packages.yml
# on each release. Do not edit them manually.

class TraderClaw < Formula
  desc "Rust crypto trading agent for EVM, Solana, TON, and Polymarket"
  homepage "https://github.com/Trader-Claw-Labs/Trader-Claw"
  version "__VERSION__"
  license "MIT OR Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/Trader-Claw-Labs/Trader-Claw/releases/download/v__VERSION__/trader-claw-macos-arm64.tar.gz"
      sha256 "__SHA256_MACOS_ARM64__"
    end
    on_intel do
      url "https://github.com/Trader-Claw-Labs/Trader-Claw/releases/download/v__VERSION__/trader-claw-macos-x86_64.tar.gz"
      sha256 "__SHA256_MACOS_X86_64__"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Trader-Claw-Labs/Trader-Claw/releases/download/v__VERSION__/trader-claw-linux-arm64.tar.gz"
      sha256 "__SHA256_LINUX_ARM64__"
    end
    on_intel do
      url "https://github.com/Trader-Claw-Labs/Trader-Claw/releases/download/v__VERSION__/trader-claw-linux-x86_64.tar.gz"
      sha256 "__SHA256_LINUX_X86_64__"
    end
  end

  def install
    bin.install "trader-claw"
  end

  def caveats
    <<~EOS
      Trader Claw requires a config file at ~/.traderclaw/config.toml
      To get started, run:
        trader-claw gateway

      The web dashboard will be available at http://localhost:42617
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/trader-claw --version")
  end
end
