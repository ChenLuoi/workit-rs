import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

const pkg = JSON.parse(readFileSync('package.json', 'utf8'));
const registry = 'https://registry.npmjs.org';
const names = [...Object.keys(pkg.optionalDependencies), pkg.name];
const files = names.map(name => {
  const stem = name.replace(/^@/, '').replace('/', '-');
  const file = join(process.argv[2], stem + '-' + pkg.version + '.tgz');
  const integrity = 'sha512-' + createHash('sha512').update(readFileSync(file)).digest('base64');
  return { name, file, integrity };
});

// Publish the launcher last. A retry may reuse only the same bytes.
for (const { name, file, integrity } of files) {
  const url = registry + '/' + name.replace('/', '%2f') + '/' + pkg.version;
  const res = await fetch(url);
  if (res.ok) {
    assert.equal((await res.json()).dist.integrity, integrity, name + ' already exists with different contents');
    console.log('Already published ' + name + '@' + pkg.version);
  } else {
    assert.equal(res.status, 404, 'registry lookup failed for ' + name);
    execFileSync('npm', ['publish', file, '--registry', registry, '--access', 'public',
      '--tag', pkg.version.includes('-') ? 'next' : 'latest'], { stdio: 'inherit' });
  }
}
