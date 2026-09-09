#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ "$(uname -s)" != "Darwin" ]]; then
  printf '%s\n' 'Localoud AI currently supports the macOS Tauri build.' >&2
  exit 1
fi

for command_name in cargo node npm xcrun; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    printf 'Required command not found: %s\n' "$command_name" >&2
    exit 1
  fi
done

if ! xcrun --find clang >/dev/null 2>&1; then
  printf '%s\n' 'Xcode command line tools are required. Install them with: xcode-select --install' >&2
  exit 1
fi

cd "$repo_root/apps/desktop"
printf '%s\n' 'Installing frontend dependencies...'
npm_cache_dir="$repo_root/.npm-cache"
if [[ -n "${LOCALOUD_NPM_CACHE:-}" ]]; then
  npm_cache_dir="$LOCALOUD_NPM_CACHE"
fi
mkdir -p "$npm_cache_dir"
NPM_CONFIG_CACHE="$npm_cache_dir" npm ci

printf '%s\n' 'Building the unsigned debug app bundle...'
npm run tauri -- build --debug --no-sign --bundles app

app_path="$repo_root/target/debug/bundle/macos/Localoud AI.app"
if [[ ! -d "$app_path" ]]; then
  printf 'Build completed without the expected app bundle: %s\n' "$app_path" >&2
  exit 1
fi

printf '\nInstalled locally at:\n%s\n' "$app_path"
printf 'Open it with: open "%s"\n' "$app_path"
