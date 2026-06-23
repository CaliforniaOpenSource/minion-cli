#!/bin/sh
set -eu

REPO="${MINION_HUB_REPO:-CaliforniaOpenSource/minion-cli}"
VERSION="${MINION_HUB_VERSION:-latest}"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
BIN_NAME="minion-hub"
CHECKSUMS_NAME="minion-hub-checksums.txt"
TOKEN="${GITHUB_TOKEN:-${GH_TOKEN:-}}"

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: required command not found: $1" >&2
    exit 1
  fi
}

need curl
need grep
need id
need install
need tar
need sha256sum
need uname
need mktemp

download() {
  if [ -n "$TOKEN" ]; then
    curl -fsSL \
      -H "Authorization: Bearer ${TOKEN}" \
      -H "X-GitHub-Api-Version: 2022-11-28" \
      "$@"
  else
    curl -fsSL "$@"
  fi
}

os="$(uname -s)"
if [ "$os" != "Linux" ]; then
  echo "error: minion-hub installer only supports Linux" >&2
  exit 1
fi

case "$(uname -m)" in
  x86_64 | amd64)
    arch="amd64"
    ;;
  aarch64 | arm64)
    arch="arm64"
    ;;
  *)
    echo "error: unsupported architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

asset="minion-hub-linux-${arch}.tar.gz"
if [ "$VERSION" = "latest" ]; then
  download_base="https://github.com/${REPO}/releases/latest/download"
else
  download_base="https://github.com/${REPO}/releases/download/${VERSION}"
fi

tmp_dir="$(mktemp -d)"
cleanup() {
  rm -rf "$tmp_dir"
}
trap cleanup EXIT INT TERM

echo "Downloading ${asset} from ${REPO} (${VERSION})..."
download -o "${tmp_dir}/${asset}" "${download_base}/${asset}"
download -o "${tmp_dir}/${CHECKSUMS_NAME}" "${download_base}/${CHECKSUMS_NAME}"

(
  cd "$tmp_dir"
  checksum_line="$(grep "  ${asset}\$" "$CHECKSUMS_NAME" || true)"
  if [ -z "$checksum_line" ]; then
    echo "error: checksum for ${asset} not found in ${CHECKSUMS_NAME}" >&2
    exit 1
  fi
  printf '%s\n' "$checksum_line" | sha256sum -c -
)

tar -xzf "${tmp_dir}/${asset}" -C "$tmp_dir" "$BIN_NAME"
chmod +x "${tmp_dir}/${BIN_NAME}"

if [ "$(id -u)" -eq 0 ]; then
  install -d "$INSTALL_DIR"
  install -m 0755 "${tmp_dir}/${BIN_NAME}" "${INSTALL_DIR}/${BIN_NAME}"
else
  need sudo
  sudo install -d "$INSTALL_DIR"
  sudo install -m 0755 "${tmp_dir}/${BIN_NAME}" "${INSTALL_DIR}/${BIN_NAME}"
fi

echo "Installed ${INSTALL_DIR}/${BIN_NAME}"
echo "Next: sudo ${BIN_NAME} init"
