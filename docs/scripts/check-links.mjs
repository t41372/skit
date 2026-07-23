// Post-build link checker for the static export.
//
// Crawls every HTML file in out/ and verifies that each internal link and asset
// reference resolves to a real file, and that every #fragment matches an id on
// its target page. Absolute internal links must carry the /skit base path (or
// they break on GitHub Pages). Exits non-zero on any problem.
import { readFileSync, readdirSync, statSync, existsSync } from 'node:fs';
import { dirname, join, relative, resolve, posix } from 'node:path';
import { fileURLToPath } from 'node:url';

const OUT = resolve(dirname(fileURLToPath(import.meta.url)), '../out');
const BASE = '/skit';

if (!existsSync(OUT)) {
  console.error(`No build output at ${OUT} — run \`npm run build\` first.`);
  process.exit(2);
}

const htmlFiles = [];
(function walk(dir) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) walk(p);
    else if (name.endsWith('.html')) htmlFiles.push(p);
  }
})(OUT);

function resolveTarget(urlPath) {
  const rel = urlPath.replace(/^\/+/, '');
  for (const c of [join(OUT, rel), join(OUT, rel, 'index.html'), join(OUT, `${rel}.html`)]) {
    if (existsSync(c) && statSync(c).isFile()) return c;
  }
  return null;
}

const idCache = new Map();
function idsOf(file) {
  if (idCache.has(file)) return idCache.get(file);
  const html = readFileSync(file, 'utf8');
  const ids = new Set();
  for (const m of html.matchAll(/\b(?:id|name)="([^"]+)"/g)) ids.add(m[1]);
  idCache.set(file, ids);
  return ids;
}

const problems = [];

for (const file of htmlFiles) {
  const html = readFileSync(file, 'utf8');
  const pageRel = `/${relative(OUT, file).split(/[\\/]/).join('/')}`;
  // Boundary before href/src excludes data-href, xlink:href, and friends.
  for (const [, raw] of html.matchAll(/(?<![\w:-])(?:href|src)="([^"]*)"/g)) {
    if (!raw || /^(https?:|mailto:|tel:|data:|javascript:|\/\/)/i.test(raw)) continue;

    let path = raw;
    let frag = '';
    const h = path.indexOf('#');
    if (h >= 0) [path, frag] = [path.slice(0, h), path.slice(h + 1)];
    const q = path.indexOf('?');
    if (q >= 0) path = path.slice(0, q);

    if (!path) {
      // same-page anchor
      if (frag && !idsOf(file).has(decodeURIComponent(frag))) {
        problems.push(`${pageRel}: dead same-page anchor #${frag}`);
      }
      continue;
    }

    let urlPath;
    if (path.startsWith('/')) {
      if (path === BASE || path.startsWith(`${BASE}/`)) {
        urlPath = path.slice(BASE.length) || '/';
      } else {
        problems.push(`${pageRel}: internal link missing ${BASE} base path → ${raw}`);
        continue;
      }
    } else {
      urlPath = posix.normalize(posix.join(posix.dirname(pageRel), path));
    }

    const target = resolveTarget(urlPath);
    if (!target) {
      problems.push(`${pageRel}: broken link → ${raw}`);
    } else if (frag && target.endsWith('.html') && !idsOf(target).has(decodeURIComponent(frag))) {
      problems.push(`${pageRel}: dead anchor → ${raw}`);
    }
  }
}

if (problems.length) {
  console.error(`✗ ${problems.length} link problem(s):`);
  for (const p of problems.sort()) console.error(`  ${p}`);
  process.exit(1);
}
console.log(`✓ checked ${htmlFiles.length} pages — every internal link and anchor resolves.`);
