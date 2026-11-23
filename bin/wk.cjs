#!/usr/bin/env node
const { spawn } = require('node:child_process');
const { constants } = require('node:os');

let target = process.platform + '-' + process.arch;
if (process.platform === 'linux') {
  target += process.report.getReport().header.glibcVersionRuntime ? '-gnu' : '-musl';
} else if (process.platform === 'win32') {
  target += '-msvc';
}
const pkg = '@workit-rs/' + target;
if (!Object.hasOwn(require('../package.json').optionalDependencies, pkg)) {
  console.error('wk: unsupported npm platform ' + target);
  process.exit(1);
}
let binary;
try {
  binary = require.resolve(pkg + '/bin/' + (process.platform === 'win32' ? 'wk.exe' : 'wk'));
} catch {
  console.error('wk: missing ' + pkg + '. Reinstall workit-rs with npm --include=optional.');
  process.exit(1);
}
const child = spawn(binary, process.argv.slice(2), { argv0: 'wk', stdio: 'inherit' });
for (const signal of ['SIGHUP', 'SIGINT', 'SIGTERM']) {
  process.on(signal, () => child.kill(signal));
}
child.on('error', error => {
  console.error('wk: ' + error.message);
  process.exitCode = 1;
});
child.on('exit', (code, signal) => {
  process.exitCode = code ?? (128 + constants.signals[signal]);
});
