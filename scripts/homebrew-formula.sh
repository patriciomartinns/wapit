#!/bin/sh
# Prints the Homebrew formula for a release from its SHA256SUMS file.
# Usage: scripts/homebrew-formula.sh v0.1.0 SHA256SUMS
set -eu

tag="$1"
sums="$2"
version="${tag#v}"
base="https://github.com/patriciomartinns/wapit/releases/download/${tag}"

sha() {
    awk -v file="wapit-${tag}-$1.tar.gz" '$2 == file { print $1 }' "${sums}"
}

for target in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-musl x86_64-unknown-linux-musl; do
    if [ -z "$(sha "${target}")" ]; then
        echo "missing checksum for ${target}" >&2
        exit 1
    fi
done

cat <<FORMULA
class Wapit < Formula
  desc "Local WhatsApp Cloud API mock and inbox for development"
  homepage "https://github.com/patriciomartinns/wapit"
  version "${version}"
  license "MIT"

  on_macos do
    on_arm do
      url "${base}/wapit-${tag}-aarch64-apple-darwin.tar.gz"
      sha256 "$(sha aarch64-apple-darwin)"
    end
    on_intel do
      url "${base}/wapit-${tag}-x86_64-apple-darwin.tar.gz"
      sha256 "$(sha x86_64-apple-darwin)"
    end
  end

  on_linux do
    on_arm do
      url "${base}/wapit-${tag}-aarch64-unknown-linux-musl.tar.gz"
      sha256 "$(sha aarch64-unknown-linux-musl)"
    end
    on_intel do
      url "${base}/wapit-${tag}-x86_64-unknown-linux-musl.tar.gz"
      sha256 "$(sha x86_64-unknown-linux-musl)"
    end
  end

  def install
    bin.install "wapit"
  end

  service do
    run [opt_bin/"wapit"]
    environment_variables WAPIT_CONFIG_DIR: etc/"wapit", WAPIT_DATA_DIR: var/"wapit"
    keep_alive true
    log_path var/"log/wapit.log"
    error_log_path var/"log/wapit.log"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/wapit --version")
  end
end
FORMULA
