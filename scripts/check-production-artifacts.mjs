import { readdir, readFile } from 'node:fs/promises';
import path from 'node:path';

const directory = path.resolve('dist/assets');
const files = (await readdir(directory)).filter(name => /\.(js|css)$/.test(name));
if (files.length === 0) throw new Error('No production assets found');
const forbidden = ['__ORBIT_TEST__', 'SYNTHETIC_SECRET_SHOULD_NOT_RENDER', 'orv_sk_SYNTHETIC_BROWSER_SECRET', 'orv_sk_UNIT_TEST_SECRET', 'Private synthetic account A prompt', 'unsupported_fixture_command', 'connection-synthetic-account-a'];
for (const name of files) {
  const content = await readFile(path.join(directory, name), 'utf8');
  for (const marker of forbidden) if (content.includes(marker)) throw new Error(`Test-only marker present in production artifact: ${marker}`);
}
console.log(`Production fixture exclusion verified in ${files.length} assets.`);
