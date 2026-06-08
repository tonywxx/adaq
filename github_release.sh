#!/usr/bin/env bash
set -euo pipefail

REPO="${GITHUB_REPOSITORY:-tonywxx/adaq}"
KEY_FILE="${TAURI_SIGNING_PRIVATE_KEY_FILE:-$HOME/.tauri/adaq-updater.key}"
VERSION="${1:-}"

usage() {
	cat <<'EOF'
Usage:
  ./github_release.sh <version>

Example:
  export TAURI_SIGNING_PRIVATE_KEY_PASSWORD='your updater key password'
  ./github_release.sh 0.1.3

What it does:
  1. Updates package.json, src-tauri/Cargo.toml, and src-tauri/tauri.conf.json.
  2. Builds signed Tauri bundles with createUpdaterArtifacts enabled.
  3. Generates release/latest.json for all detected updater artifacts.
  4. Commits, tags, pushes, and creates/uploads the GitHub release.

Notes:
  - This script uploads artifacts for the OS it is run on.
  - Run it on macOS, Linux, and Windows CI runners to publish all platforms.
  - It never prints TAURI_SIGNING_PRIVATE_KEY.
EOF
}

if [[ "${VERSION}" == "-h" || "${VERSION}" == "--help" ]]; then
	usage
	exit 0
fi

if [[ -z "${VERSION}" ]]; then
	usage >&2
	exit 1
fi

if [[ ! "${VERSION}" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-.][0-9A-Za-z.-]+)?$ ]]; then
	echo "Invalid version: ${VERSION}" >&2
	exit 1
fi

require_cmd() {
	if ! command -v "$1" >/dev/null 2>&1; then
		echo "Missing required command: $1" >&2
		exit 1
	fi
}

require_cmd git
require_cmd gh
require_cmd node
require_cmd pnpm

if ! git diff --quiet || ! git diff --cached --quiet; then
	echo "Working tree has uncommitted changes. Commit/stash them before releasing." >&2
	exit 1
fi

if ! gh auth status >/dev/null 2>&1; then
	echo "GitHub CLI is not authenticated. Run: gh auth login -h github.com" >&2
	exit 1
fi

if [[ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
	if [[ ! -f "${KEY_FILE}" ]]; then
		echo "Missing updater key file: ${KEY_FILE}" >&2
		echo "Set TAURI_SIGNING_PRIVATE_KEY or TAURI_SIGNING_PRIVATE_KEY_FILE." >&2
		exit 1
	fi

	TAURI_SIGNING_PRIVATE_KEY="$(cat "${KEY_FILE}")"
	export TAURI_SIGNING_PRIVATE_KEY
fi

if [[ -z "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" ]]; then
	echo "Missing TAURI_SIGNING_PRIVATE_KEY_PASSWORD." >&2
	exit 1
fi

TAG="v${VERSION}"
RELEASE_DIR="release"
BUNDLE_DIR="src-tauri/target/release/bundle"
LATEST_JSON="${RELEASE_DIR}/latest.json"

if gh release view "${TAG}" --repo "${REPO}" >/dev/null 2>&1; then
	echo "Release ${TAG} already exists on ${REPO}." >&2
	exit 1
fi

if git rev-parse "${TAG}" >/dev/null 2>&1; then
	echo "Local tag ${TAG} already exists." >&2
	exit 1
fi

remote_tag="$(git ls-remote --tags origin "${TAG}" || true)"
if [[ -n "${remote_tag}" ]]; then
	echo "Remote tag ${TAG} already exists." >&2
	exit 1
fi

echo "Updating project version to ${VERSION}"
node -e "
const fs = require('fs')
for (const file of ['package.json', 'src-tauri/tauri.conf.json']) {
  const json = JSON.parse(fs.readFileSync(file, 'utf8'))
  json.version = process.argv[1]
  fs.writeFileSync(file, JSON.stringify(json, null, file === 'package.json' ? 2 : '\t') + '\n')
}
const cargoToml = 'src-tauri/Cargo.toml'
const cargo = fs.readFileSync(cargoToml, 'utf8')
fs.writeFileSync(cargoToml, cargo.replace(/^version = \"[^\"]+\"/m, 'version = \"' + process.argv[1] + '\"'))
" "${VERSION}"

echo "Building signed Tauri bundles"
pnpm tauri build

mkdir -p "${RELEASE_DIR}"

echo "Generating ${LATEST_JSON}"
node - "${VERSION}" "${TAG}" "${LATEST_JSON}" "${BUNDLE_DIR}" "${REPO}" <<'EOF'
const fs = require('fs')
const path = require('path')

const version = process.argv[2]
const tag = process.argv[3]
const latestPath = process.argv[4]
const bundleDir = process.argv[5]
const repo = process.argv[6]

const archMap = new Map([
  ['x64', 'x86_64'],
  ['ia32', 'i686'],
  ['arm64', 'aarch64'],
  ['arm', 'armv7'],
])

function walk(dir) {
  if (!fs.existsSync(dir)) return []
  const entries = fs.readdirSync(dir, { withFileTypes: true })
  return entries.flatMap((entry) => {
    const fullPath = path.join(dir, entry.name)
    return entry.isDirectory() ? walk(fullPath) : [fullPath]
  })
}

function currentPlatformKey(file) {
  const normalized = file.split(path.sep).join('/')
  const arch = archMap.get(process.arch) || process.arch

  if (normalized.includes('/macos/')) return `darwin-${arch}`
  if (normalized.includes('/linux/')) return `linux-${arch}`
  if (normalized.includes('/msi/') || normalized.includes('/nsis/')) {
    return `windows-${arch}`
  }

  return null
}

function unsignedArtifactName(sigPath) {
  return path.basename(sigPath).replace(/\.sig$/, '')
}

const signatures = walk(bundleDir).filter((file) => file.endsWith('.sig'))
const platforms = {}

for (const sigPath of signatures) {
  const platform = currentPlatformKey(sigPath)
  if (!platform) continue

  const artifact = unsignedArtifactName(sigPath)
  const signature = fs.readFileSync(sigPath, 'utf8').trim()

  platforms[platform] = {
    signature,
    url: `https://github.com/${repo}/releases/download/${tag}/${artifact}`,
  }
}

if (!Object.keys(platforms).length) {
  throw new Error(`No updater signature files found under ${bundleDir}`)
}

const latest = {
  version,
  notes: `Release ${tag}`,
  pub_date: new Date().toISOString(),
  platforms,
}

fs.writeFileSync(latestPath, `${JSON.stringify(latest, null, '\t')}\n`)
EOF

echo "Collecting release assets"
asset_list="$(mktemp)"

find "${BUNDLE_DIR}" -type f -name "*.sig" -print | while IFS= read -r sig; do
	printf '%s\n' "${sig}" >>"${asset_list}"
	unsigned="${sig%.sig}"
	if [[ -f "${unsigned}" ]]; then
		printf '%s\n' "${unsigned}" >>"${asset_list}"
	fi
done

find "${BUNDLE_DIR}" -type f \( \
	-name "*.dmg" -o \
	-name "*.AppImage" -o \
	-name "*.deb" -o \
	-name "*.rpm" -o \
	-name "*.msi" -o \
	-name "*.exe" \
\) -print | while IFS= read -r installer; do
	if [[ "$(basename "${installer}")" == *"${VERSION}"* ]]; then
		printf '%s\n' "${installer}" >>"${asset_list}"
	fi
done

printf '%s\n' "${LATEST_JSON}" >>"${asset_list}"

assets=()
while IFS= read -r asset; do
	assets+=("${asset}")
done < <(sort -u "${asset_list}")
rm -f "${asset_list}"

printf 'Assets:\n'
printf '  %s\n' "${assets[@]}"

echo "Validating latest.json"
node -e "
const fs = require('fs')
const latest = JSON.parse(fs.readFileSync(process.argv[1], 'utf8'))
if (latest.version !== process.argv[2]) throw new Error('latest.json version mismatch')
for (const [platform, entry] of Object.entries(latest.platforms || {})) {
  if (!entry.url || !entry.signature) throw new Error('Invalid platform entry: ' + platform)
}
console.log('latest.json ok')
" "${LATEST_JSON}" "${VERSION}"

echo "Committing release changes"
git add package.json pnpm-lock.yaml src-tauri/Cargo.lock src-tauri/Cargo.toml src-tauri/tauri.conf.json "${LATEST_JSON}"
git commit -m "Release ${TAG}"

echo "Pushing main and tag ${TAG}"
git push origin main
git tag "${TAG}"
git push origin "${TAG}"

echo "Creating GitHub release ${TAG}"
gh release create "${TAG}" \
	--repo "${REPO}" \
	--title "${TAG}" \
	--notes "Release ${TAG}" \
	"${assets[@]}"

echo "Release published: https://github.com/${REPO}/releases/tag/${TAG}"
