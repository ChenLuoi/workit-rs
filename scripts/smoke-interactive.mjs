import assert from 'node:assert/strict';
import { execFileSync, spawn, spawnSync } from 'node:child_process';
import { once } from 'node:events';
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, renameSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { stripVTControlCharacters } from 'node:util';

const wk = resolve(process.argv[2]);
const env = {
  ...process.env,
  GIT_CONFIG_NOSYSTEM: '1',
  GIT_CONFIG_GLOBAL: '/dev/null',
  GIT_TERMINAL_PROMPT: '0',
  SHELL: '/bin/sh',
  TERM: 'xterm',
  CLICOLOR: '1',
  CLICOLOR_FORCE: '0',
};
delete env.NO_COLOR;
const root = realpathSync(mkdtempSync(join(process.platform === 'darwin' ? '/tmp' : tmpdir(), 'wk-interactive-')));
const space = join(root, "wk's workspace");
const repo = join(root, 'sample');
const bin = join(root, 'bin');
const manifest = () => readFileSync(join(space, 'wk.yaml'), 'utf8');

function git(dir, ...args) {
  return execFileSync('git', ['-C', dir, ...args], { env, encoding: 'utf8' }).trim();
}

function commit(message) {
  git(repo, '-c', 'user.name=test', '-c', 'user.email=test@example.org',
    'commit', '--allow-empty', '-qm', message);
}

async function interact(args, steps = [], expectedCode = 0, workspace = space, cols = 120) {
  const command = ['env', 'PATH=' + bin, wk, '-w', workspace, ...args]
    .map(arg => "'" + arg.replaceAll("'", "'\\''") + "'").join(' ');
  // Run each command in a controlling terminal with a known size.
  const session = `stty rows 30 cols ${cols} && ${command}; printf '\\nWK_EXIT=%s\\n' "$?"`;
  writeFileSync(join(root, 'session.sh'), session + '\n');
  const child = spawn('socat', ['STDIO', 'EXEC:/bin/sh session.sh,pty,setsid,ctty,stderr'], {
    env, cwd: root, timeout: 15_000, killSignal: 'SIGKILL',
  });
  let log = '';
  let diagnostic = '';
  let offset = 0;
  let next = 0;
  child.stderr.setEncoding('utf8').on('data', chunk => { diagnostic += chunk; });
  child.stdout.setEncoding('utf8').on('data', chunk => {
    log += chunk;
    if (/\nWK_EXIT=\d+\r?\n/.test(log)) child.stdin.end();
    const step = steps[next];
    if (step && log.includes(step[0], offset)) {
      offset = log.length;
      next++;
      child.stdin.write(typeof step[1] === 'function' ? step[1]() : step[1]);
    }
  });
  const [code, signal] = await once(child, 'close');
  assert.equal(code, 0, `PTY exited (${signal || code}) during ${args.join(' ')}\n${diagnostic}\n${log}`);
  assert.equal(next, steps.length, `Missing prompt: ${steps[next]?.[0]}\n${log}`);
  assert.equal(log.match(/\nWK_EXIT=(\d+)\r?\n/)?.[1], String(expectedCode), log);
  return log;
}

function checkTask(name, expected) {
  const record = JSON.parse(execFileSync(wk, ['-w', space, 'status', name, '--json'], { env, encoding: 'utf8' })).tasks[name];
  assert.deepEqual(Object.keys(record.projects).sort(), Object.keys(expected).sort());
  for (const [project, base] of Object.entries(expected)) {
    const tree = join(space, 'tasks', name, project);
    const sha = git(join(space, 'repos', project), 'rev-parse', base);
    assert.deepEqual(record.projects[project].start, { base, commit: sha, ...(project === 'alt' ? { description: 'API producer' } : {}) });
    assert.equal(git(tree, 'rev-parse', 'HEAD'), sha);
    assert.equal(git(tree, 'branch', '--show-current'), name);
  }
}

try {
  mkdirSync(bin);
  const gitPath = execFileSync('/bin/sh', ['-c', 'command -v git'], { encoding: 'utf8' }).trim();
  symlinkSync(gitPath, join(bin, 'git'));
  function installTool(tool) {
    writeFileSync(join(bin, tool), `#!/bin/sh\nprintf 'launched ${tool} in %s\\n' "$PWD"\n`, { mode: 0o755 });
  }
  for (const tool of ['codex', 'agy', 'code']) installTool(tool);
  git(root, 'init', '-q', '-b', 'trunk', repo);
  commit('initial');
  for (const branch of ['feature/release', 'release/1.x', 'release/2.x']) {
    git(repo, 'branch', branch);
  }
  const clone = ['clone', repo];
  const enter = '\r';
  const down = '\x1b[B';
  const clearSelections = '\x1b[D'; // Left arrow in a multiselect.
  const esc = '\x1b';
  const confirmTarget = `Initialize workspace in ${space}?`;
  const firstProject = [[confirmTarget, enter], ['First project URL', repo + enter]];
  const declined = await interact(['init'], [[confirmTarget, 'n' + enter]], 1);
  assert.doesNotMatch(declined, /First project URL/);
  assert.ok(!existsSync(space));
  await interact(['init'], [[confirmTarget, enter], ['First project URL', esc]], 1);
  assert.ok(!existsSync(space));
  await interact(['init'], [...firstProject, ['Project directory', enter], ['Task base', esc]], 1);
  assert.ok(!existsSync(space));

  const emptySpace = join(root, 'empty workspace');
  const emptySetup = [[`Initialize workspace in ${emptySpace}?`, enter], ['First project URL', enter]];
  await interact(['init'], [...emptySetup, ['Enable tools', esc]], 1, emptySpace);
  assert.ok(!existsSync(emptySpace));
  const skillPicker = await interact(['init'], [
    ['Initialize workspace in', enter], ['First project URL', enter], ['Enable tools', enter], ['Install skills', esc],
  ], 1, emptySpace, 48);
  const menu = stripVTControlCharacters(skillPicker).replace(/[\r\n]/g, '');
  assert.match(menu, /wk-cli\s+Look up wk commands.*managing a workspace\./);
  assert.match(menu, /wk-workspace-workflow\s+Scope a user request.*from the workspace\./);
  assert.match(menu, /wk-worktree-optimization\s+Review registered projects.*pnpm migrations\./);
  assert.ok(!existsSync(emptySpace));
  const skipped = await interact(['init'], [
    ...emptySetup, ['Enable tools', enter], ['Install skills', clearSelections + enter],
  ], 0, emptySpace);
  assert.match(skipped, /✓ Preparing workspace directory.*✓ Writing workspace files/s);
  assert.ok(skipped.includes('\x1b[2mclaude (not installed)'));
  env.NO_COLOR = '1';
  const editTools = ['Shared settings to edit', ' ' + enter];
  const emptyManifest = readFileSync(join(emptySpace, 'wk.yaml'), 'utf8');
  const savedDefaults = await interact(['setup', '--shared'], [editTools, ['Enable tools', enter]], 0, emptySpace);
  delete env.NO_COLOR;
  assert.equal(readFileSync(join(emptySpace, 'wk.yaml'), 'utf8'), emptyManifest);
  assert.match(emptyManifest, /tools:[\s\S]*?\bcodex\b[\s\S]*?\bagy\b/);
  assert.ok(savedDefaults.includes('claude (not installed)'));
  assert.ok(!savedDefaults.includes('\x1b[2mclaude (not installed)'));
  assert.doesNotMatch(skipped, /Project directory|Reading remote branches|Task base|Cloning|Registered/);
  assert.deepEqual(readdirSync(join(emptySpace, 'repos')), ['.gitignore']);
  assert.ok(!existsSync(join(emptySpace, '.agents')));
  execFileSync(wk, ['-w', emptySpace, 'doctor'], { env });
  await interact(['setup'], [['Configuration scope', esc]], 1, emptySpace);
  await interact(['setup'], [
    ['Configuration scope', enter], ['Personal settings to edit', enter],
    ['Local projects', enter], ['Local tools', esc],
  ], 1, emptySpace);
  assert.ok(!existsSync(join(emptySpace, '.wk', 'local.yaml')));
  const sharedSetup = await interact(['setup'], [
    ['Configuration scope', down + enter], editTools, ['Enable tools', enter],
  ], 0, emptySpace);
  assert.doesNotMatch(sharedSetup, /Personal settings to edit/);
  const localSetup = await interact(['setup', '--local'], [
    ['Personal settings to edit', enter], ['Local projects', enter], ['Local tools', enter],
  ], 0, emptySpace);
  assert.doesNotMatch(localSetup, /Configuration scope/);
  assert.ok(existsSync(join(emptySpace, '.wk', 'local.yaml')));
  const scopedShared = await interact(['setup', '--shared'], [['Shared settings to edit', esc]], 1, emptySpace);
  assert.doesNotMatch(scopedShared, /Configuration scope/);
  assert.equal(readFileSync(join(emptySpace, 'wk.yaml'), 'utf8'), emptyManifest);
  const emptyStart = await interact(['start'], [], 1, emptySpace);
  assert.match(emptyStart, /wk add <url>/);
  assert.doesNotMatch(emptyStart, /Task name|Select projects/);
  const emptyRun = await interact(['run'], [], 1, emptySpace);
  assert.match(emptyRun, /no active tasks.*wk start/);
  assert.doesNotMatch(emptyRun, /Launch tool/);

  mkdirSync(space);
  const marker = join(space, '.keep');
  await interact(['init'], [...firstProject, ['Project directory', enter], ['Task base', () => {
    writeFileSync(marker, 'keep');
    return enter;
  }], ['Project description', enter], ['Enable tools', enter], ['Install skills', enter]], 1);
  assert.deepEqual(readdirSync(space), ['.keep']);
  const occupied = await interact(['init'], [], 1);
  assert.match(occupied, /not empty/);
  assert.ok(!occupied.includes(confirmTarget));
  rmSync(marker);

  const offline = join(root, 'offline');
  const failed = await interact(['init'], [...firstProject, ['Project directory', enter], ['Task base', () => {
    renameSync(repo, offline);
    return enter;
  }], ['Project description', enter], ['Enable tools', enter], ['Install skills', enter]], 1);
  assert.match(failed, /✗ Cloning sample/);
  assert.deepEqual(readdirSync(space), []);
  renameSync(offline, repo);
  installTool('claude');
  const initialized = await interact(['init'], [
    [confirmTarget, enter], ['First project URL', '-' + enter], ['Enter a Git URL or leave it blank', '\x7f' + repo + enter],
    ['Project directory', 'alt' + enter], ['Task base', 'rel' + down + enter], ['Project description', 'API producer' + enter], ['Enable tools', enter],
    ['Install skills', enter],
  ]);
  assert.match(initialized, /This directory will hold.*Initialize workspace in/s);
  assert.match(initialized, /leave it blank to add a project later.*First project URL/s);
  assert.match(initialized, /its directory under repos\/.*Project directory/s);
  assert.match(initialized, /start from the selected branch.*Task base/s);
  assert.match(initialized, /✓ Preparing workspace directory.*✓ Cloning alt.*✓ Writing workspace files.*Registered alt/s);
  assert.ok(existsSync(join(space, 'repos', 'alt', '.git')));
  for (const name of ['wk-cli', 'wk-workspace-workflow', 'wk-worktree-optimization']) {
    assert.ok(existsSync(join(space, '.agents', 'skills', name, 'SKILL.md')));
  }
  assert.match(manifest(), /base: origin\/release\/2\.x/);

  const log = await interact(clone, [['Project directory', enter], ['Task base', enter], ['Project description', enter]]);
  assert.match(log, /Reading remote branches.*Task base.*Cloning sample.*Registered sample/s);
  assert.ok(existsSync(join(space, 'repos', 'sample', '.git')));
  assert.match(manifest(), /base: origin\/trunk/);

  const before = manifest();
  execFileSync(wk, ['-w', space, 'start', 'damaged', 'alt', '--no-fetch'], { env });
  renameSync(join(space, 'tasks/damaged/.git'), join(space, 'tasks/damaged/broken.git'));
  const declinedClose = await interact(['close', 'damaged'], [['Clean up this damaged task?', enter]], 1);
  assert.match(declinedClose, /Warning: damaged task damaged/);
  assert.match(declinedClose, /All remaining files under .*damaged will be permanently deleted/);
  assert.ok(existsSync(join(space, 'tasks/damaged/alt/.git')));
  await interact(['close', 'damaged'], [['Clean up this damaged task?', 'y' + enter]]);
  assert.ok(!existsSync(join(space, 'tasks/damaged')));
  assert.ok(!existsSync(join(space, '.wk/recovery')));
  const cancelled = await interact(clone, [
    ['Project directory', 'cancelled' + enter], ['Task base', 'elease' + enter + esc],
  ], 1);
  assert.equal(manifest(), before);
  assert.ok(!existsSync(join(space, 'repos', 'cancelled')));
  assert.doesNotMatch(cancelled, /Cloning cancelled/);

  const status = spawnSync(wk, ['-w', space, 'status', '--fetch', '--json'], { env, encoding: 'utf8' });
  assert.equal(status.status, 0, status.stderr);
  assert.equal(status.stderr, '');
  assert.deepEqual(JSON.parse(status.stdout).projects.alt, {
    name: 'alt', url: repo, description: 'API producer', base: 'origin/release/2.x', commit: git(repo, 'rev-parse', 'release/2.x'),
  });

  const both = ['Select projects', ' ' + down + ' ' + enter];
  const confirm = 'Use configured default bases?';
  const defaults = await interact(['start', 'defaults'], [both, [confirm, enter]]);
  assert.match(stripVTControlCharacters(defaults), /alt — API producer/);
  assert.doesNotMatch(defaults, /Task base for/);
  checkTask('defaults', { alt: 'origin/release/2.x', sample: 'origin/trunk' });
  assert.match(defaults, /run defaults --agent codex/);
  assert.match(defaults, /run defaults --agent claude/);
  assert.match(defaults, /run defaults --agent agy/);
  assert.doesNotMatch(defaults, /--agent (vscode|zed)/);
  const selected = await interact(['run', 'defaults'], [['Launch tool', enter]]);
  assert.ok(selected.includes('launched codex in ' + join(space, 'tasks', 'defaults')));
  const configBefore = manifest();
  await interact(['setup', '--shared'], [editTools, ['Enable tools', esc]], 1);
  assert.equal(manifest(), configBefore);
  await interact(['setup', '--shared'], [editTools, ['Enable tools', ' ' + down + ' ' + enter]]);
  const edited = manifest();
  await interact(['setup', '--shared'], [editTools, ['Enable tools', enter]]);
  assert.equal(manifest(), edited);
  const changed = await interact(['run', 'defaults'], [['Launch tool', enter]]);
  assert.ok(changed.includes('launched agy in ' + join(space, 'tasks', 'defaults')));
  assert.doesNotMatch(changed, /codex|claude|vscode|zed/);
  assert.match(await interact(['run', 'defaults', '--codex'], [], 1), /not enabled/);
  const wide = stripVTControlCharacters(await interact(['status'], [], 0, space, 220));
  assert.match(wide, /Name +Snapshot +Task base +Remote/);
  assert.match(wide, /└─ sample +defaults +✓ clean/);
  const narrow = stripVTControlCharacters(await interact(['status'], [], 0, space, 38));
  assert.match(narrow, /  alt\r?\n +Snapshot +[0-9a-f]{12}\r?\n +Task base +origin\/release\/2\.x/);
  assert.ok(narrow.includes(repo));

  git(repo, 'switch', '-qc', 'feature/new');
  commit('new base');
  git(repo, 'switch', '-q', 'trunk');
  const custom = { alt: 'origin/feature/new', sample: 'origin/release/2.x' };
  const cancelledName = await interact(['start'], [['Task name', esc]], 1);
  assert.doesNotMatch(cancelledName, /Select projects/);
  await interact(['start'], [
    ['Task name', enter], ['Enter a task name with valid', 'feature/custom' + enter], both, [confirm, 'n' + enter],
    ['Task base for alt', 'feature/n' + enter], ['Task base for sample', 'rel' + down + enter],
  ]);
  checkTask('feature/custom', custom);

  renameSync(repo, join(root, 'disconnected'));
  const one = await interact(['start', 'one', '-D', '--no-fetch'], [
    ['Select projects', enter], ['Select at least one project', '  ' + down + ' ' + enter],
  ]);
  assert.ok(!one.includes(confirm));
  checkTask('one', { sample: 'origin/trunk' });

  const activeBefore = readFileSync(join(space, '.wk', 'active.yaml'), 'utf8');
  const aborted = await interact(['start', 'aborted', 'alt', 'sample', '--no-fetch'], [
    [confirm, 'n' + enter], ['Task base for alt', 'trunk' + enter], ['Task base for sample', esc],
  ], 1);
  assert.doesNotMatch(aborted, /Fetching/);
  assert.equal(readFileSync(join(space, '.wk', 'active.yaml'), 'utf8'), activeBefore);
  assert.ok(!existsSync(join(space, 'tasks', 'aborted')));
  for (const project of ['alt', 'sample']) {
    assert.equal(git(join(space, 'repos', project), 'branch', '--list', 'aborted'), '');
  }

  const resumed = await interact(['start'], [['Task name', 'feature/custom' + enter]]);
  assert.doesNotMatch(resumed, /Fetching|Select projects/);
  checkTask('feature/custom', custom);

  for (const id of ['contract', 'integration']) {
    const dir = join(space, 'task-skills', id);
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, 'SKILL.md'), `---\nname: ${id}\ndescription: Verify ${id} changes\n---\nCheck the task worktrees.\n`);
  }
  const created = await interact(['preset', 'add', 'pair'], [
    ['Preset description', 'Joint development' + enter], both,
    ['Base override for alt', 'origin/feature/new' + enter], ['Base override for sample', enter],
    ['Select task skills', ' ' + enter],
  ]);
  assert.match(stripVTControlCharacters(created), /Verify contract changes/);
  const presetManifest = manifest();
  const exact = await interact(['start', 'pair-exact', '--preset', 'pair', '--no-fetch']);
  assert.doesNotMatch(exact, /Select projects|Select task skills|Task preset|Use configured/);
  checkTask('pair-exact', { alt: 'origin/feature/new', sample: 'origin/trunk' });
  const skillsAt = task => readdirSync(join(space, 'tasks', task, '.agents', 'skills'));
  assert.deepEqual(skillsAt('pair-exact'), ['contract']);
  await interact(['start', 'pair-adjusted', '--preset', 'pair', '--interactive', '--no-fetch'], [
    ['Select projects', down + ' ' + enter], [confirm, 'n' + enter],
    ['Task base for alt', 'trunk' + enter], ['Select task skills', ' ' + down + ' ' + enter],
  ]);
  checkTask('pair-adjusted', { alt: 'origin/trunk' });
  assert.deepEqual(skillsAt('pair-adjusted'), ['integration']);
  assert.equal(manifest(), presetManifest);
  await interact(['start', 'from-menu', '--no-fetch'], [
    ['Task preset', down + enter], ['Select projects', enter], [confirm, enter], ['Select task skills', enter],
  ]);
  checkTask('from-menu', { alt: 'origin/feature/new', sample: 'origin/trunk' });
  assert.deepEqual(skillsAt('from-menu'), ['contract']);
  await interact(['preset', 'edit', 'pair'], [
    ['Preset description', enter], ['Select projects', ' ' + enter],
    ['Base override for sample', enter], ['Select task skills', ' ' + down + ' ' + enter],
  ]);
  await interact(['start', 'after-edit', '--preset', 'pair', '--no-fetch']);
  checkTask('after-edit', { sample: 'origin/trunk' });
  assert.deepEqual(skillsAt('after-edit'), ['integration']);
  await interact(['start', 'cancel-preset', '--preset', 'pair', '-i', '--no-fetch'], [
    ['Select projects', enter], [confirm, enter], ['Select task skills', esc],
  ], 1);
  assert.ok(!existsSync(join(space, 'tasks', 'cancel-preset')));
} finally {
  rmSync(root, { recursive: true, force: true });
}
