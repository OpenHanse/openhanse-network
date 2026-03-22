#!/bin/zsh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SOURCE_DIR="$SCRIPT_DIR"
BUILD_DIR="$SCRIPT_DIR/Artefact"
CRATE_BINARY_NAME="openhanse_gateway_cli"
ARTIFACT_NAME="openhanse-cli"
BUILD_SYSTEM_DIR="$REPO_ROOT/BuildSystem"
RUST_IMAGE_NAME="rust-linux"
RUST_IMAGE_VERSION="1.94.0"
CONTAINER_MEMORY="${OPENHANSE_CLI_CONTAINER_MEMORY:-${OPENHANSE_HUB_CONTAINER_MEMORY:-6G}}"
CONTAINER_CPUS="${OPENHANSE_CLI_CONTAINER_CPUS:-${OPENHANSE_HUB_CONTAINER_CPUS:-4}}"
CONTAINER_CARGO_JOBS="${OPENHANSE_CLI_CARGO_JOBS:-${OPENHANSE_HUB_CARGO_JOBS:-2}}"
TARGETS=(
  "aarch64-apple-darwin"
  "x86_64-unknown-linux-gnu"
  "aarch64-unknown-linux-gnu"
)
RUSTUP_BIN="$HOME/.cargo/bin"

if [ -x "$RUSTUP_BIN/cargo" ] && [ -x "$RUSTUP_BIN/rustc" ]; then
  export PATH="$RUSTUP_BIN:$PATH"
fi

if [[ -n "${OPENHANSE_CLI_TARGETS:-}" ]]; then
  TARGETS=("${(@s:,:)OPENHANSE_CLI_TARGETS}")
elif [[ -n "${OPENHANSE_HUB_TARGETS:-}" ]]; then
  TARGETS=("${(@s:,:)OPENHANSE_HUB_TARGETS}")
fi

mkdir -p "$BUILD_DIR"

normalize_target() {
  local value="$1"
  case "$value" in
    macos-apple-silicon) echo "aarch64-apple-darwin" ;;
    linux-x86_64) echo "x86_64-unknown-linux-gnu" ;;
    linux-aarch64) echo "aarch64-unknown-linux-gnu" ;;
    *) echo "$value" ;;
  esac
}

target_label() {
  local target="$1"
  case "$target" in
    aarch64-apple-darwin) echo "macos-apple-silicon" ;;
    x86_64-unknown-linux-gnu) echo "linux-x86_64" ;;
    aarch64-unknown-linux-gnu) echo "linux-aarch64" ;;
    *) echo "$target" ;;
  esac
}

container_arch() {
  local target="$1"
  case "$target" in
    x86_64-unknown-linux-gnu) echo "amd64" ;;
    aarch64-unknown-linux-gnu) echo "arm64" ;;
    *) return 1 ;;
  esac
}

require_rust_target() {
  local target="$1"
  local label="$2"
  if ! rustup target list --installed | grep -qx "$target"; then
    echo "Missing Rust target for $label" >&2
    echo "Install it with: rustup target add $target" >&2
    exit 1
  fi
}

build_macos_target() {
  local target="$1"
  local label="$2"
  local source_artifact="$SOURCE_DIR/target/$target/release/$CRATE_BINARY_NAME"
  local target_artifact="$BUILD_DIR/$ARTIFACT_NAME-$label"

  require_rust_target "$target" "$label"

  echo "Building $ARTIFACT_NAME for $label..."
  cargo build --release --target "$target" --manifest-path "$SOURCE_DIR/Cargo.toml"

  cp "$source_artifact" "$target_artifact"
  chmod +x "$target_artifact"
  echo "Build artifact written to $target_artifact"
}

build_linux_target() {
  local target="$1"
  local label="$2"
  local arch
  local image_tag
  local target_dir_host
  local source_artifact
  local target_artifact
  local container_name
  local image_archive

  arch="$(container_arch "$target")"
  image_tag="${RUST_IMAGE_NAME}:${RUST_IMAGE_VERSION}-${arch}"
  image_archive="$BUILD_SYSTEM_DIR/Artefact/${RUST_IMAGE_NAME}_${RUST_IMAGE_VERSION}_${arch}.oci.tar"
  target_dir_host="$SOURCE_DIR/target/container-$label"
  source_artifact="$target_dir_host/$target/release/$CRATE_BINARY_NAME"
  target_artifact="$BUILD_DIR/$ARTIFACT_NAME-$label"
  container_name="$ARTIFACT_NAME-build-$label"

  if ! command -v container >/dev/null 2>&1; then
    echo "Missing Apple Container CLI: install it with 'brew install container'" >&2
    exit 1
  fi

  ensure_container_system_running

  ensure_build_image_exists "$image_tag" "$image_archive" "$arch"

  rm -rf "$target_dir_host"

  echo "Building $ARTIFACT_NAME for $label in container..."
  container run \
    --rm \
    --name "$container_name" \
    --arch "$arch" \
    --memory "$CONTAINER_MEMORY" \
    --cpus "$CONTAINER_CPUS" \
    --env "CARGO_TARGET_DIR=/workspace/openhanse-network/Source/openhanse-cli/target/container-$label" \
    --env "CARGO_BUILD_JOBS=$CONTAINER_CARGO_JOBS" \
    --volume "$REPO_ROOT:/workspace/openhanse-network" \
    --workdir /workspace/openhanse-network/Source/openhanse-cli \
    "$image_tag" \
    cargo build --release --target "$target"

  if [ ! -f "$source_artifact" ]; then
    echo "Expected build artifact missing: $source_artifact" >&2
    exit 1
  fi

  cp "$source_artifact" "$target_artifact"
  chmod +x "$target_artifact"
  echo "Build artifact written to $target_artifact"
}

ensure_container_system_running() {
  if container list --all >/dev/null 2>&1; then
    return 0
  fi

  echo "Starting Apple Container system..."
  container system start >/dev/null

  if ! container list --all >/dev/null 2>&1; then
    echo "Apple Container system could not be started." >&2
    exit 1
  fi
}

ensure_build_image_exists() {
  local image_tag="$1"
  local image_archive="$2"
  local arch="$3"

  if container image inspect "$image_tag" >/dev/null 2>&1; then
    return 0
  fi

  echo "Missing build image: $image_tag" >&2
  echo "Expected prebuilt image archive: $image_archive" >&2
  echo "Build it first with:" >&2
  echo "  cd $BUILD_SYSTEM_DIR && ./BuildContainerImage.sh $RUST_IMAGE_NAME $RUST_IMAGE_VERSION $arch" >&2
  exit 1
}

NORMALIZED_TARGETS=()
for target in "${TARGETS[@]}"; do
  NORMALIZED_TARGETS+=("$(normalize_target "$target")")
done

for target in "${NORMALIZED_TARGETS[@]}"; do
  label="$(target_label "$target")"
  case "$target" in
    aarch64-apple-darwin)
      build_macos_target "$target" "$label"
      ;;
    x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu)
      build_linux_target "$target" "$label"
      ;;
    *)
      echo "Unsupported target: $target" >&2
      exit 1
      ;;
  esac
done
