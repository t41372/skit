// The docs landing page (content/docs/index.mdx) renders the repo README via
// Fumadocs' <include>. But <include> — and Turbopack — can only resolve files
// INSIDE the docs/ project root, and README.md lives one level up. So copy it
// into docs/.generated/ before dev and build, and include that in-root copy.
//
// Runs from package.json's predev/prebuild hooks; the copy is gitignored.
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const currentDir = dirname(fileURLToPath(import.meta.url));
const src = resolve(currentDir, '../../README.md');
const dest = resolve(currentDir, '../.generated/readme.md');

mkdirSync(dirname(dest), { recursive: true });
copyFileSync(src, dest);
console.log(`Synced README → ${dest.replace(resolve(currentDir, '..') + '/', '')}`);
