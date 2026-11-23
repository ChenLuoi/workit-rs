import assert from 'node:assert/strict';
import { execFileSync, spawn, spawnSync } from 'node:child_process';
import { once } from 'node:events';
import { existsSync, mkdtempSync, readFileSync, renameSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const wk = resolve(process.argv[2]);
const env = {
  ...process.env,
  GIT_CONFIG_NOSYSTEM: '1',
  GIT_CONFIG_GLOBAL: '/dev/null',
  GIT_TERMINAL_PROMPT: '0',
  SHELL: '/bin/sh',
  TERM: 'xterm',
};
const root = mkdtempSync(join(tmpdir(), 'wk-interactive-'));
const space = join(root, "wk's workspace");
const repo = join(root, 'sample');
const manifest = () => readFileSync(join(space, 'wk.yaml'), 'utf8');

function git(dir, ...args) {
  return execFileSync('git', ['-C', dir, ...args], { env, encoding: 'utf8' }).trim();
}

function commit(message) {
  git(repo, '-c', 'user.name=test', '-c', 'user.email=test@example.org',
    'commit', '--allow-empty', '-qm', message);
}

async function interact(args, steps = [], expectedCode = 0) {
  const command = [wk, '-w', space, ...args]
    .map(arg => "'" + arg.replaceAll("'", "'\\''") + "'").join(' ');
  // Report wk's exit status inside the PTY on both BSD and util-linux script.
  const session = `stty rows 30 cols 120 && ${command}; printf '\\nWK_EXIT=%s\\n' "$?"`;
  const scriptArgs = process.platform === 'linux'
    ? ['-q', '-c', session, '/dev/null']
    : ['-q', '/dev/null', '/bin/sh', '-c', session];
  const child = spawn('script', scriptArgs, { env, timeout: 15_000 });
  let log = '';
  let diagnostic = '';
  let offset = 0;
  let next = 0;
  child.stderr.setEncoding('utf8').on('data', chunk => { diagnostic += chunk; });
  child.stdout.setEncoding('utf8').on('data', chunk => {
    log += chunk;
    const step = steps[next];
    if (step && log.includes(step[0], offset)) {
      offset = log.length;
      next++;
      child.stdin.write(step[1]);
    }
  });
  const [code, signal] = await once(child, 'close');
  assert.equal(code, 0, `script exited (${signal || code})\n${diagnostic}\n${log}`);
  assert.equal(next, steps.length, `Missing prompt: ${steps[next]?.[0]}\n${log}`);
  assert.equal(log.match(/\nWK_EXIT=(\d+)\r?\n/)?.[1], String(expectedCode), log);
  return log;
}

function checkTask(name, expected) {
  const record = JSON.parse(readFileSync(join(space, '.wk', 'tasks', name + '.json'), 'utf8'));
  assert.deepEqual(Object.keys(record.projects).sort(), Object.keys(expected).sort());
  for (const [project, base] of Object.entries(expected)) {
    const tree = join(space, 'tasks', name, project);
    const sha = git(join(space, 'repos', project), 'rev-parse', base);
    assert.deepEqual(record.projects[project], { base, commit: sha });
    assert.equal(git(tree, 'rev-parse', 'HEAD'), sha);
    assert.equal(git(tree, 'branch', '--show-current'), 'wk/' + name);
  }
}

try {
  git(root, 'init', '-q', '-b', 'trunk', repo);
  commit('initial');
  for (const branch of ['feature/release', 'release/1.x', 'release/2.x']) {
    git(repo, 'branch', branch);
  }
  execFileSync(wk, ['-w', space, 'init'], { env });

  const clone = ['clone', repo];
  const enter = '\r';
  const down = '\x1b[B';
  const esc = '\x1b';
  const log = await interact(clone, [['Project directory', enter], ['Task base', enter]]);
  assert.match(log, /Reading remote branches.*Task base.*Cloning sample.*Registered sample/s);
  assert.ok(existsSync(join(space, 'repos', 'sample', '.git')));
  assert.match(manifest(), /base: origin\/trunk/);

  await interact(clone, [['Project directory', 'alt' + enter], ['Task base', 'rel' + down + enter]]);
  assert.ok(existsSync(join(space, 'repos', 'alt', '.git')));
  assert.match(manifest(), /base: origin\/release\/2\.x/);

  const before = manifest();
  const cancelled = await interact(clone, [
    ['Project directory', 'cancelled' + enter], ['Task base', 'elease' + enter + esc],
  ], 1);
  assert.equal(manifest(), before);
  assert.ok(!existsSync(join(space, 'repos', 'cancelled')));
  assert.doesNotMatch(cancelled, /Cloning cancelled/);

  const status = spawnSync(wk, ['-w', space, 'status', '--fetch', '--json'], { env, encoding: 'utf8' });
  assert.equal(status.status, 0, status.stderr);
  assert.equal(status.stderr, '');
  assert.ok(Object.hasOwn(JSON.parse(status.stdout).projects, 'sample'));

  const both = ['Select projects', ' ' + down + ' ' + enter];
  const confirm = 'Use configured default bases?';
  const defaults = await interact(['start', 'defaults'], [both, [confirm, enter]]);
  assert.doesNotMatch(defaults, /Task base for/);
  checkTask('defaults', { alt: 'origin/release/2.x', sample: 'origin/trunk' });

  git(repo, 'switch', '-qc', 'feature/new');
  commit('new base');
  git(repo, 'switch', '-q', 'trunk');
  const custom = { alt: 'origin/feature/new', sample: 'origin/release/2.x' };
  await interact(['start', 'custom'], [
    both, [confirm, 'n' + enter],
    ['Task base for alt', 'feature/n' + enter], ['Task base for sample', 'rel' + down + enter],
  ]);
  checkTask('custom', custom);

  renameSync(repo, join(root, 'disconnected'));
  const one = await interact(['start', 'one', '-D', '--no-fetch'], [
    ['Select projects', enter], ['Select at least one project', '  ' + down + ' ' + enter],
  ]);
  assert.ok(!one.includes(confirm));
  checkTask('one', { sample: 'origin/trunk' });

  const aborted = await interact(['start', 'aborted', 'alt', 'sample', '--no-fetch'], [
    [confirm, 'n' + enter], ['Task base for alt', 'trunk' + enter], ['Task base for sample', esc],
  ], 1);
  assert.doesNotMatch(aborted, /Fetching/);
  assert.ok(!existsSync(join(space, '.wk', 'tasks', 'aborted.json')));
  assert.ok(!existsSync(join(space, 'tasks', 'aborted')));
  for (const project of ['alt', 'sample']) {
    assert.equal(git(join(space, 'repos', project), 'branch', '--list', 'wk/aborted'), '');
  }

  const resumed = await interact(['start', 'custom']);
  assert.doesNotMatch(resumed, /Fetching|Select projects/);
  checkTask('custom', custom);
} finally {
  rmSync(root, { recursive: true, force: true });
}
