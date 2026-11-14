#!/usr/bin/env node
// Re-run the landing-page snippets against the toolchain built in this repo and rewrite their
// .out.txt files, so the output shown on the site is always real.
//   cargo build --release   (in the repo root), then:   npm run snippets
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const ROOT = path.join(path.dirname(fileURLToPath(import.meta.url)), '..');
const SNIP = path.join(ROOT, 'src/snippets');
const bin = process.env.TULPA_BIN || ['tulpa', 'tulpa.exe'].map((b) => path.join(ROOT, '..', 'target/release', b)).find((p) => fs.existsSync(p));
if (!bin) { console.error('no toolchain: run `cargo build --release` in the repo root, or set TULPA_BIN'); process.exit(2); }

const runs = { 'hero.rav': [], 'units.rav': [], 'contracts.rav': ['verify'], 'timing.rav': ['--virtual-time'] };
for (const [file, pre] of Object.entries(runs)) {
  const r = spawnSync(bin, [...pre, file], { cwd: SNIP, encoding: 'utf8' });
  fs.writeFileSync(path.join(SNIP, file.replace('.rav', '.out.txt')), r.stdout + r.stderr);
  console.log(`${file}: exit ${r.status}`);
}
console.log('cosim.out.txt and morty.out.txt need HermiSim + MuJoCo and are refreshed by hand.');
