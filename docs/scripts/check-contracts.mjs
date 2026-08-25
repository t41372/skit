// Check documentation contracts that do not need a built site.
import { existsSync, readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const read = (path) => readFileSync(resolve(root, path), 'utf8');
const problems = [];

function requireText(path, text) {
  if (!read(path).includes(text)) problems.push(`${path} must contain ${text}`);
}

function refuseText(path, text) {
  if (read(path).includes(text)) problems.push(`${path} must not contain ${text}`);
}

refuseText('docs/app/api/search/route.ts', "language: 'english'");
requireText('docs/lib/source.ts', 'page.locale');

for (const path of [
  'docs/app/llms.mdx/[lang]/docs/[[...slug]]/route.ts',
  'docs/app/og/docs/[lang]/[...slug]/route.tsx',
]) {
  if (!existsSync(resolve(root, path))) problems.push(`${path} is missing`);
}

for (const locale of ['zh-CN', 'zh-TW']) {
  requireText('.github/workflows/docs.yml', `README.${locale}.md`);
  requireText(`README.${locale}.md`, '[![CodSpeed]');
}

const paramsFlags = [
  '--binding',
  '--multiple',
  '--no-multiple',
  '--repeat',
  '--no-repeat',
  '--env-target',
  '--action',
];
const globalFlags = ['--data-dir', '--install-completion', '--show-completion'];
for (const path of [
  'docs/content/docs/cli.mdx',
  'docs/content/docs/cli.zh-CN.mdx',
  'docs/content/docs/cli.zh-TW.mdx',
]) {
  for (const flag of [...paramsFlags, ...globalFlags]) requireText(path, flag);
}

for (const path of [
  'README.zh-CN.md',
  'README.zh-TW.md',
  'docs/content/docs/prompts.zh-CN.mdx',
  'docs/content/docs/prompts.zh-TW.mdx',
  'docs/content/docs/script-types.zh-CN.mdx',
  'docs/content/docs/script-types.zh-TW.mdx',
]) {
  for (const token of ['{{占位符}}', '{{佔位符}}', '{{預留位置}}', '{{洞}}']) {
    refuseText(path, token);
  }
}

requireText('docs/content/docs/cli.zh-CN.mdx', '[#skit-list--show]');
requireText('docs/content/docs/cli.zh-CN.mdx', '[#skit-remove--rename--describe--edit]');
requireText('docs/content/docs/cli.zh-TW.mdx', '[#skit-list--show]');
requireText('docs/content/docs/cli.zh-TW.mdx', '[#skit-remove--rename--describe--edit]');
refuseText('docs/README.md', 'English-only for now');

const skill = read('skills/skit/SKILL.md');
if (/\p{Script=Han}/u.test(skill)) problems.push('skills/skit/SKILL.md must be English-only');
for (const meaning of ['interpreter', 'runtime', 'runner']) {
  if (!skill.includes(meaning)) problems.push(`skills/skit/SKILL.md must explain exit 126 ${meaning}`);
}

if (problems.length > 0) {
  console.error(`Found ${problems.length} documentation contract problem(s):`);
  for (const problem of problems) console.error(`- ${problem}`);
  process.exit(1);
}

console.log('Documentation source contracts pass.');
