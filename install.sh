#!/bin/sh
set -eu

fail() {
  printf 'tnpm installer: %s\n' "$*" >&2
  exit 1
}

[ -n "${HOME:-}" ] || fail 'HOME is not set'

case "$(uname -s)" in
  Linux) os=unknown-linux-gnu ;;
  Darwin) os=apple-darwin ;;
  *) fail 'Only Linux (glibc) and macOS are supported' ;;
esac

case "$(uname -m)" in
  x86_64 | amd64) arch=x86_64 ;;
  arm64 | aarch64) arch=aarch64 ;;
  *) fail 'No official binary for this CPU architecture' ;;
esac

if [ "$os" = apple-darwin ] && [ "$arch" = aarch64 ]; then
  asset=tnpm-aarch64-apple-darwin
else
  asset="tnpm-$arch-$os"
fi

install_dir=${TNPM_INSTALL_DIR:-"$HOME/.local/bin"}
case "$install_dir" in
  /*) ;;
  *) fail 'TNPM_INSTALL_DIR must be an absolute path' ;;
esac

command -v awk >/dev/null 2>&1 || fail 'awk is required'
if command -v curl >/dev/null 2>&1; then
  fetch() { curl --fail --location --silent --show-error --retry 3 --output "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
  fetch() { wget --quiet --tries=3 --output-document="$2" "$1"; }
else
  fail 'curl or wget is required'
fi

tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/tnpm-install.XXXXXXXX") || fail 'Cannot create temporary directory'
candidate=
cleanup() {
  [ -z "$candidate" ] || rm -f "$candidate"
  rm -rf "$tmp_dir"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM

base=https://github.com/taisan11/tnpm/releases/latest/download
printf 'Downloading %s...\n' "$asset"
fetch "$base/$asset" "$tmp_dir/tnpm" || fail 'Binary download failed'
fetch "$base/SHA256SUMS" "$tmp_dir/SHA256SUMS" || fail 'Checksum download failed'

expected=$(awk -v file="$asset" '
  ($2 == file || $2 == "*" file) && length($1) == 64 && $1 !~ /[^0-9a-fA-F]/ {
    sum = tolower($1)
    count++
  }
  END {
    if (count != 1) exit 1
    print sum
  }
' "$tmp_dir/SHA256SUMS") || fail 'Missing or duplicated SHA-256 checksum'

if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$tmp_dir/tnpm" | awk '{print tolower($1)}')
elif command -v shasum >/dev/null 2>&1; then
  actual=$(shasum -a 256 "$tmp_dir/tnpm" | awk '{print tolower($1)}')
elif command -v openssl >/dev/null 2>&1; then
  actual=$(openssl dgst -sha256 "$tmp_dir/tnpm" | awk '{print tolower($NF)}')
else
  fail 'sha256sum, shasum, or openssl is required for verification'
fi
[ "$actual" = "$expected" ] || fail 'SHA-256 mismatch; nothing was installed'

chmod 755 "$tmp_dir/tnpm"
"$tmp_dir/tnpm" --help >/dev/null 2>&1 || fail 'Downloaded tnpm cannot run on this system (check glibc and architecture)'

mkdir -p "$install_dir" || fail "Cannot create $install_dir"
candidate="$install_dir/.tnpm-install.$$"
cp "$tmp_dir/tnpm" "$candidate"
chmod 755 "$candidate"
mv -f "$candidate" "$install_dir/tnpm"
candidate=
printf 'Installed tnpm to %s/tnpm\n' "$install_dir"

case ":${PATH:-}:" in
  *":$install_dir:"*) ;;
  *) printf 'Add %s to your PATH (for example, in your shell profile).\n' "$install_dir" ;;
esac
