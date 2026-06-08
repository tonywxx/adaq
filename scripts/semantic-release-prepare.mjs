import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'

const [version, tag] = process.argv.slice(2)

if (!version || !tag) {
  throw new Error('Usage: semantic-release-prepare.mjs <version> <tag>')
}

if (!/^[0-9]+\.[0-9]+\.[0-9]+(?:[-.][0-9A-Za-z.-]+)?$/.test(version)) {
  throw new Error(`Invalid release version: ${version}`)
}

const repo = process.env.GITHUB_REPOSITORY || 'tonywxx/adaq'
const bundleDir = 'src-tauri/target/release/bundle'
const latestPath = 'release/latest.json'

updateJson('package.json', version, 2)
updateJson('src-tauri/tauri.conf.json', version, '\t')
updateCargoVersion('src-tauri/Cargo.toml', version)

execFileSync('pnpm', ['install', '--lockfile-only'], { stdio: 'inherit' })
execFileSync('pnpm', ['tauri', 'build'], { stdio: 'inherit' })

fs.mkdirSync(path.dirname(latestPath), { recursive: true })
writeLatestJson({
  version,
  tag,
  latestPath,
  bundleDir,
  repo,
})
validateLatestJson(latestPath, version)

function updateJson(file, nextVersion, indent) {
  const json = JSON.parse(fs.readFileSync(file, 'utf8'))
  json.version = nextVersion
  fs.writeFileSync(file, `${JSON.stringify(json, null, indent)}\n`)
}

function updateCargoVersion(file, nextVersion) {
  const cargo = fs.readFileSync(file, 'utf8')
  const nextCargo = cargo.replace(/^version = "[^"]+"/m, `version = "${nextVersion}"`)

  if (nextCargo === cargo) {
    throw new Error(`Could not update version in ${file}`)
  }

  fs.writeFileSync(file, nextCargo)
}

function writeLatestJson({ version, tag, latestPath, bundleDir, repo }) {
  const platforms = {}

  for (const sigPath of walk(bundleDir).filter((file) => file.endsWith('.sig'))) {
    const platform = currentPlatformKey(sigPath)
    if (!platform) continue

    const artifact = path.basename(sigPath).replace(/\.sig$/, '')
    platforms[platform] = {
      signature: fs.readFileSync(sigPath, 'utf8').trim(),
      url: `https://github.com/${repo}/releases/download/${tag}/${artifact}`,
    }
  }

  if (!Object.keys(platforms).length) {
    throw new Error(`No updater signature files found under ${bundleDir}`)
  }

  fs.writeFileSync(
    latestPath,
    `${JSON.stringify(
      {
        version,
        notes: `Release ${tag}`,
        pub_date: new Date().toISOString(),
        platforms,
      },
      null,
      '\t',
    )}\n`,
  )
}

function validateLatestJson(latestPath, expectedVersion) {
  const latest = JSON.parse(fs.readFileSync(latestPath, 'utf8'))

  if (latest.version !== expectedVersion) {
    throw new Error('latest.json version mismatch')
  }

  for (const [platform, entry] of Object.entries(latest.platforms || {})) {
    if (!entry.url || !entry.signature) {
      throw new Error(`Invalid platform entry: ${platform}`)
    }
  }
}

function walk(dir) {
  if (!fs.existsSync(dir)) return []

  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
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

const archMap = new Map([
  ['x64', 'x86_64'],
  ['ia32', 'i686'],
  ['arm64', 'aarch64'],
  ['arm', 'armv7'],
])
