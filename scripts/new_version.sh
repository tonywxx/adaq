#!/usr/bin/env bash

set -euo pipefail

script_dir=$(cd -- "$(dirname -- "$0")" && pwd)
root_dir=$(dirname "$script_dir")

if [[ $# -eq 0 ]]; then
  current=$(node -p "require('$root_dir/package.json').version")
  if [[ ! $current =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "Error: invalid version in package.json ($current)" >&2
    exit 1
  fi
  IFS=. read -r major minor patch <<< "$current"
  version="$major.$minor.$((patch + 1))"
elif [[ $# -eq 1 && $1 =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  version="$1"
else
  echo "Usage: $0 [X.Y.Z]" >&2
  exit 1
fi

node --input-type=module - "$version" "$root_dir" <<'NODE'
import fs from 'node:fs'
import path from 'node:path'

const [version, root] = process.argv.slice(2)
const files = {
  package: path.join(root, 'package.json'),
  cargo: path.join(root, 'src-tauri/Cargo.toml'),
  tauri: path.join(root, 'src-tauri/tauri.conf.json'),
}

const packageJson = JSON.parse(fs.readFileSync(files.package, 'utf8'))
const tauriConfig = JSON.parse(fs.readFileSync(files.tauri, 'utf8'))
const cargo = fs.readFileSync(files.cargo, 'utf8')

if (!/^\[package\]\n(?:[^\n]*\n)*?version = "[^"]+"/m.test(cargo)) {
  throw new Error(`Could not find the package version in ${files.cargo}`)
}

packageJson.version = version
tauriConfig.version = version
const updatedCargo = cargo.replace(
  /^(\[package\]\n(?:[^\n]*\n)*?version = ")[^"]+(")/m,
  `$1${version}$2`,
)

fs.writeFileSync(files.package, `${JSON.stringify(packageJson, null, '\t')}\n`)
fs.writeFileSync(files.tauri, `${JSON.stringify(tauriConfig, null, '\t')}\n`)
fs.writeFileSync(files.cargo, updatedCargo)

const versions = [
  JSON.parse(fs.readFileSync(files.package, 'utf8')).version,
  /^version = "([^"]+)"/m.exec(fs.readFileSync(files.cargo, 'utf8'))?.[1],
  JSON.parse(fs.readFileSync(files.tauri, 'utf8')).version,
]
if (!versions.every((actual) => actual === version)) {
  throw new Error(`Version sync failed: ${versions.join(', ')}`)
}

console.log(`Set package.json, Cargo.toml, and tauri.conf.json to ${version}`)
NODE
