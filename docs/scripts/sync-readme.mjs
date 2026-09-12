// The docs landing page (content/docs/index.mdx and its translations) renders the
// repo README via Fumadocs' <include>. But <include> — and Turbopack — can only
// resolve files INSIDE the docs/ project root, and the READMEs live one level up.
// So copy each one into docs/.generated/ before dev and build, and include those
// in-root copies.
//
// Runs from package.json's predev/prebuild hooks; the copies are gitignored.
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const currentDir = dirname(fileURLToPath(import.meta.url));
const root = resolve(currentDir, '..');

// One entry per docs locale. The key is the generated file name.
const readmes = {
  'readme.md': 'README.md',
  'readme.zh-CN.md': 'README.zh-CN.md',
  'readme.zh-TW.md': 'README.zh-TW.md',
};

for (const [name, source] of Object.entries(readmes)) {
  const src = resolve(root, '..', source);
  const dest = resolve(root, '.generated', name);
  mkdirSync(dirname(dest), { recursive: true });
  copyFileSync(src, dest);
  console.log(`Synced ../${source} → ${dest.replace(`${root}/`, '')}`);
}
