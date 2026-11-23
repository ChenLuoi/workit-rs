import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { chmodSync, copyFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const pkg = JSON.parse(readFileSync('package.json', 'utf8'));
const targets = [
  ['linux-x64-gnu', 'x86_64-unknown-linux-gnu', 'ubuntu-22.04'],
  ['linux-arm64-gnu', 'aarch64-unknown-linux-gnu', 'ubuntu-24.04-arm'],
  ['linux-x64-musl', 'x86_64-unknown-linux-musl', 'ubuntu-22.04'],
  ['linux-arm64-musl', 'aarch64-unknown-linux-musl', 'ubuntu-24.04-arm'],
  ['darwin-x64', 'x86_64-apple-darwin', 'macos-15-intel'],
  ['darwin-arm64', 'aarch64-apple-darwin', 'macos-15'],
  ['win32-x64-msvc', 'x86_64-pc-windows-msvc', 'windows-latest'],
].map(([target, rust, runner]) => ({
  target, rust, runner, binary: target.startsWith('win32') ? 'wk.exe' : 'wk',
}));
const platformName = target => '@workit-rs/' + target;
const [cmd, arg, binary, dest] = process.argv.slice(2);

switch (cmd) {
  case 'check': {
    const meta = JSON.parse(execFileSync('cargo', ['metadata', '--locked', '--no-deps', '--format-version', '1']));
    const crate = meta.packages.find(p => p.name === 'workit-rs');
    assert.ok(crate?.targets.some(t => t.kind.includes('bin') && t.name === 'wk'));
    assert.equal(pkg.name, crate.name);
    assert.equal(pkg.version, crate.version);
    assert.equal(pkg.bin.wk, 'bin/wk.cjs');
    assert.deepEqual(pkg.optionalDependencies, Object.fromEntries(targets.map(t => [platformName(t.target), pkg.version])));
    if (arg) assert.equal(arg, 'v' + pkg.version, 'tag must match Cargo and npm versions');
    break;
  }
  case 'matrix':
    console.log(JSON.stringify({ include: targets }));
    break;
  case 'stage': {
    const target = targets.find(t => t.target === arg);
    assert.ok(target && binary && dest, 'usage: stage <target> <built-binary> <package-dir>');
    const [os, cpu, abi] = target.target.split('-');
    const manifest = {
      name: platformName(arg), version: pkg.version, license: pkg.license,
      description: pkg.description + ' (' + arg + ')',
      os: [os], cpu: [cpu], files: ['bin/', 'LICENSE'],
      ...(os === 'linux' && {libc: [abi === 'gnu' ? 'glibc' : 'musl']}),
    };
    mkdirSync(join(dest, 'bin'), { recursive: true });
    const exe = join(dest, 'bin', target.binary);
    copyFileSync(binary, exe);
    chmodSync(exe, 0o755);
    copyFileSync('LICENSE', join(dest, 'LICENSE'));
    writeFileSync(join(dest, 'package.json'), JSON.stringify(manifest, null, 2) + '\n');
    break;
  }
  default:
    throw new Error('expected check, matrix or stage');
}
