#!/usr/bin/env node
// Verifies every internal link, image and #anchor in _site/ resolves. Exits 1 on any broken reference.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const OUT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', '_site');
const files = [];
(function walk(d) { for (const e of fs.readdirSync(d, { withFileTypes: true })) { const p = path.join(d, e.name); e.isDirectory() ? walk(p) : p.endsWith('.html') && files.push(p); } })(OUT);

const ids = new Map();
for (const f of files) ids.set(f, new Set([...fs.readFileSync(f, 'utf8').matchAll(/\sid="([^"]+)"/g)].map((m) => m[1])));

let bad = 0, checked = 0;
for (const f of files) {
  if (f.endsWith('404.html')) continue; // uses absolute production URLs
  const html = fs.readFileSync(f, 'utf8');
  for (const [, attr, url] of html.matchAll(/\s(href|src)="([^"]+)"/g)) {
    if (/^(https?:|mailto:|data:|javascript:)/.test(url) || url.startsWith('//')) continue;
    checked++;
    const [p, hash] = url.split('#');
    const target = p ? path.resolve(path.dirname(f), decodeURIComponent(p)) : f;
    if (!fs.existsSync(target)) { console.log(`✗ ${path.relative(OUT, f)}: ${attr}="${url}" (missing file)`); bad++; continue; }
    if (hash && target.endsWith('.html') && !ids.get(target)?.has(decodeURIComponent(hash))) {
      console.log(`✗ ${path.relative(OUT, f)}: ${url} (missing #${hash})`); bad++;
    }
  }
}
console.log(`${checked} internal references checked, ${bad} broken`);
process.exit(bad ? 1 : 0);
