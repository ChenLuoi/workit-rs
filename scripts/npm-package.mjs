import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, copyFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { extname, join } from 'node:path';

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
const [cmd, ...args] = process.argv.slice(2);

switch (cmd) {
  case 'check': {
    const [tag] = args;
    const meta = JSON.parse(execFileSync('cargo', ['metadata', '--locked', '--no-deps', '--format-version', '1']));
    const crate = meta.packages.find(p => p.name === 'workit-rs');
    assert.ok(crate?.targets.some(t => t.kind.includes('bin') && t.name === 'wk'));
    assert.equal(pkg.name, crate.name);
    assert.equal(pkg.version, crate.version);
    assert.equal(pkg.repository.url, 'git+' + crate.repository + '.git');
    assert.equal(pkg.bin.wk, 'bin/wk.cjs');
    assert.deepEqual(pkg.optionalDependencies, Object.fromEntries(targets.map(t => [platformName(t.target), pkg.version])));
    if (tag) assert.equal(tag, 'v' + pkg.version, 'tag must match Cargo and npm versions');
    break;
  }
  case 'matrix':
    console.log(JSON.stringify({ include: targets }));
    break;
  case 'stage': {
    const [platform, binary, dest] = args;
    const target = targets.find(t => t.target === platform);
    assert.ok(target && binary && dest, 'usage: stage <target> <built-binary> <package-dir>');
    const [os, cpu, abi] = platform.split('-');
    const manifest = {
      name: platformName(platform), version: pkg.version, license: pkg.license, repository: pkg.repository,
      description: pkg.description + ' (' + platform + ')',
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
  case 'github-assets': {
    const [archiveDir, outputDir] = args;
    assert.ok(archiveDir && outputDir, 'usage: github-assets <npm-tarball-dir> <new-output-dir>');
    mkdirSync(outputDir);
    const sha256 = data => createHash('sha256').update(data).digest('hex');
    const assets = targets.map(({ target, rust, binary }) => {
      const npmPackage = platformName(target);
      const archive = join(archiveDir, `${npmPackage.slice(1).replace('/', '-')}-${pkg.version}.tgz`);
      const data = execFileSync('tar', ['-xOzf', archive, 'package/bin/' + binary], { maxBuffer: Infinity });
      assert.ok(data.length, `empty binary in ${archive}`);
      const fileName = `wk-${target}-v${pkg.version}${extname(binary)}`;
      writeFileSync(join(outputDir, fileName), data, { mode: 0o755 });
      return { kind: 'binary', fileName, bytes: data.length, sha256: sha256(data), target, npmPackage, rustTarget: rust };
    });
    const commit = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
    const manifest = JSON.stringify({ package: pkg.name, version: pkg.version, tag: 'v' + pkg.version, commit, assets }, null, 2) + '\n';
    writeFileSync(join(outputDir, 'release-manifest.json'), manifest);
    const checksums = assets.map(asset => `${asset.sha256}  ${asset.fileName}`);
    checksums.push(`${sha256(manifest)}  release-manifest.json`);
    writeFileSync(join(outputDir, 'SHA256SUMS'), checksums.join('\n') + '\n');
    break;
  }
  default:
    throw new Error('expected check, matrix, stage or github-assets');
}
