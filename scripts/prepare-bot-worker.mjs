import fs from 'node:fs'
import path from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = path.resolve(fileURLToPath(new URL('..', import.meta.url)))
const target = process.env.ADAQ_WORKER_TARGET ?? process.env.TAURI_ENV_TARGET_TRIPLE ?? hostTarget()
const profile = process.env.ADAQ_WORKER_PROFILE === 'release' ? 'release' : 'debug'
const release = profile === 'release'
const windowsTarget = target.endsWith('-pc-windows-msvc')
const binaryName = windowsTarget ? 'adaq-bot-worker.exe' : 'adaq-bot-worker'
const targetArgs = target ? ['--target', target] : []
const manifest = path.join(root, 'src-tauri', 'Cargo.toml')
const source = path.join(root, 'src-tauri', 'target', ...(target ? [target] : []), profile, binaryName)
const binaries = path.join(root, 'src-tauri', 'binaries')
const destination = path.join(binaries, `adaq-bot-worker-${target}${windowsTarget ? '.exe' : ''}`)
const signature = path.join(binaries, `adaq-bot-worker-${target}.sig`)

fs.mkdirSync(binaries, { recursive: true })
run('cargo', ['build', '--locked', '--manifest-path', manifest, '--package', 'adaq-bot-worker', ...targetArgs, ...(release ? ['--release'] : [])])
if (!fs.existsSync(source)) throw new Error(`worker artifact was not produced at ${source}`)
fs.copyFileSync(source, destination)

const signingKey = process.env.ADAQ_WORKER_SIGNING_PRIVATE_KEY_HEX
const signingKeyFile = process.env.ADAQ_WORKER_SIGNING_PRIVATE_KEY_FILE
if (!signingKey && !signingKeyFile) {
  if (process.env.ADAQ_WORKER_ALLOW_UNSIGNED !== '1') {
    throw new Error('ADAQ_WORKER_SIGNING_PRIVATE_KEY_HEX is required to sign the worker artifact')
  }
  fs.writeFileSync(signature, JSON.stringify({
    schemaVersion: 'adaq-bot-worker-signature@1.0.0',
    artifactName: 'adaq-bot-worker',
    artifactVersion: '0.1.0',
    platform: target,
    protocolVersion: 'adaq-bot-worker-ipc@1.1.0',
    runtimeVersion: 'adaq-bot-runtime@0.1.0',
    artifactSha256: 'unsigned-test-artifact',
    signingKeyId: 'adaq-bot-worker-ed25519-v1',
    signature: '',
  }, null, 2))
  console.log(`prepared unsigned test Bot Worker for ${target}`)
  process.exit(0)
}
const signingEnv = { ...process.env }
if (signingKey) signingEnv.ADAQ_WORKER_SIGNING_PRIVATE_KEY_HEX = signingKey
if (signingKeyFile) signingEnv.ADAQ_WORKER_SIGNING_PRIVATE_KEY_FILE = signingKeyFile
run('cargo', ['run', '--locked', '--manifest-path', manifest, '--package', 'adaq-bot-runtime', '--bin', 'adaq-worker-sign', ...(release ? ['--release'] : []), '--', '--artifact', destination, '--output', signature, '--platform', target], signingEnv)
console.log(`prepared signed Bot Worker for ${target}`)

function hostTarget() {
  const result = spawnSync('rustc', ['-vV'], { encoding: 'utf8' })
  if (result.status !== 0) throw new Error('could not determine the Rust host target')
  const match = result.stdout.match(/^host: (.+)$/m)
  if (!match) throw new Error('Rust host target was not reported')
  return match[1].trim()
}

function run(command, args, env = process.env) {
  const result = spawnSync(command, args, { cwd: root, env, stdio: 'inherit' })
  if (result.status !== 0) throw new Error(`${command} ${args.join(' ')} failed`)
}
