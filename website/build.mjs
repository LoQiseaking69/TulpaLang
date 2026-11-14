#!/usr/bin/env node
// Static site generator for the Tulpa & HermiSim site.
//   node build.mjs            build into _site/
//   node build.mjs --serve    build, serve on :8080 and rebuild on change
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import { fileURLToPath } from 'node:url';
import { Marked } from 'marked';
import hljs from 'highlight.js/lib/common';
import powershell from 'highlight.js/lib/languages/powershell';
import tulpa from './scripts/tulpa-hljs.mjs';

const ROOT = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.join(ROOT, '..');          // the TulpaLang checkout this site lives in
const SRC = path.join(ROOT, 'src');
const OUT = path.join(ROOT, '_site');
hljs.registerLanguage('tulpa', tulpa);
hljs.registerLanguage('powershell', powershell);

const read = (p) => fs.readFileSync(p, 'utf8');
const esc = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
const slugify = (s) => s.toLowerCase().replace(/<[^>]+>/g, '').replace(/&[a-z]+;/g, '').replace(/[`'"]/g, '')
  .replace(/[^a-z0-9À-￿]+/g, '-').replace(/^-+|-+$/g, '');

function writeOut(rel, content) {
  const p = path.join(OUT, rel);
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, content);
}
function copyDir(from, to) {
  fs.mkdirSync(to, { recursive: true });
  for (const e of fs.readdirSync(from, { withFileTypes: true })) {
    const a = path.join(from, e.name), b = path.join(to, e.name);
    e.isDirectory() ? copyDir(a, b) : fs.copyFileSync(a, b);
  }
}

export function highlight(code, lang) {
  lang = (lang || '').toLowerCase();
  if (lang === 'rav' || lang === 'tlp') lang = 'tulpa';
  if (lang === 'text' || lang === 'txt' || lang === 'console' || lang === 'mermaid') return esc(code);
  if (lang && hljs.getLanguage(lang)) return hljs.highlight(code, { language: lang, ignoreIllegals: true }).value;
  return esc(code);
}

function codeBlock(code, lang, title) {
  const label = { tulpa: 'Tulpa', rav: 'Tulpa', bash: 'Shell', sh: 'Shell', text: 'Output', json: 'JSON', toml: 'TOML',
    yaml: 'YAML', yml: 'YAML', xml: 'XML', python: 'Python', rust: 'Rust', lua: 'Lua', powershell: 'PowerShell' }[lang] || (lang || '').toUpperCase();
  const cls = lang === 'text' ? ' is-output' : '';
  return `<div class="code${cls}"><div class="code-head"><span>${esc(title || label)}</span><button class="copy" type="button" aria-label="Copy code">Copy</button></div>` +
    `<pre><code class="hljs language-${esc(lang || 'plain')}">${highlight(code.replace(/\n$/, ''), lang)}</code></pre></div>`;
}

// ---------------------------------------------------------------- config & nav
const config = JSON.parse(read(path.join(ROOT, 'site.config.json')));
const nav = JSON.parse(read(path.join(SRC, 'docs/_nav.json')));
const docPages = nav.flatMap((s) => s.pages.map((p) => ({ ...p, section: s.section })));
// site docs by file name, and repo docs by their path in the TulpaLang tree
const docSlugByFile = new Map();
const docSlugByRepoPath = new Map();
for (const p of docPages) {
  if (p.file) docSlugByFile.set(p.file, p.slug);
  if (p.repo) docSlugByRepoPath.set(p.repo, p.slug);
}

// ---------------------------------------------------------------- markdown
function makeMarked(ctx) {
  const m = new Marked({ gfm: true });
  m.use({
    renderer: {
      heading({ tokens, depth }) {
        const html = this.parser.parseInline(tokens);
        if (depth === 1) { ctx.title ??= html.replace(/<[^>]+>/g, ''); return ''; }
        let id = slugify(html) || 'section';
        const n = (ctx.ids.get(id) || 0); ctx.ids.set(id, n + 1); if (n) id += '-' + n;
        if (depth <= 3) ctx.toc.push({ depth, id, text: html.replace(/<[^>]+>/g, '') });
        ctx.paras.push(html.replace(/<[^>]+>/g, ''));
        return `<h${depth} id="${id}"><a class="anchor" href="#${id}" aria-hidden="true">#</a>${html}</h${depth}>\n`;
      },
      code({ text, lang }) { return codeBlock(text, (lang || '').split(/\s/)[0]); },
      table(token) {
        let h = '<div class="table-wrap"><table><thead><tr>';
        token.header.forEach((c, i) => { h += `<th${token.align[i] ? ` style="text-align:${token.align[i]}"` : ''}>${this.parser.parseInline(c.tokens)}</th>`; });
        h += '</tr></thead><tbody>';
        for (const row of token.rows) {
          h += '<tr>' + row.map((c, i) => `<td${token.align[i] ? ` style="text-align:${token.align[i]}"` : ''}>${this.parser.parseInline(c.tokens)}</td>`).join('') + '</tr>';
        }
        return h + '</tbody></table></div>\n';
      },
      link({ href, title, tokens }) {
        const text = this.parser.parseInline(tokens);
        const r = rewriteHref(href, ctx);
        const ext = /^https?:/.test(r) ? ' target="_blank" rel="noopener"' : '';
        return `<a href="${esc(r)}"${title ? ` title="${esc(title)}"` : ''}${ext}>${text}</a>`;
      },
      image({ href, title, text }) {
        return `<figure class="shot"><img src="${esc(href)}" alt="${esc(text)}" loading="lazy"${title ? ` title="${esc(title)}"` : ''}>${text ? `<figcaption>${esc(text)}</figcaption>` : ''}</figure>`;
      },
      blockquote({ tokens }) {
        let body = this.parser.parse(tokens);
        const m = body.match(/^<p>\[!(NOTE|TIP|WARNING|IMPORTANT|CAUTION)\]\s*/);
        if (!m) return `<blockquote>${body}</blockquote>`;
        body = body.replace(m[0], '<p>');
        const kind = m[1].toLowerCase();
        return `<div class="callout callout-${kind}"><div class="callout-title">${m[1][0] + m[1].slice(1).toLowerCase()}</div>${body}</div>`;
      },
      paragraph({ tokens }) {
        const html = this.parser.parseInline(tokens);
        ctx.paras.push(html.replace(/<[^>]+>/g, ''));
        if (/^<figure[^]*<\/figure>$/.test(html.trim())) return html + '\n';
        return `<p>${html}</p>\n`;
      }
    }
  });
  return m;
}

function rewriteHref(href, ctx) {
  if (!href || /^(https?:|mailto:|#)/.test(href)) return href;
  const [file, hash] = href.split('#');
  const tail = hash ? '#' + hash : '';
  if (ctx.repoPath) {
    // a page read from the TulpaLang tree: resolve against its location in the repo
    const rel = path.posix.normalize(path.posix.join(path.posix.dirname(ctx.repoPath), file));
    const slug = docSlugByRepoPath.get(rel);
    if (slug) return `${slug}.html${tail}`;
    return `${config.tulpa.repo}/${/\.[a-z]+$/i.test(rel) ? 'blob' : 'tree'}/main/${rel}${tail}`;
  }
  const base = file.replace(/^\.\//, '');
  if (base.endsWith('.md')) {
    const slug = docSlugByFile.get(base) || docPages.find((p) => `${p.slug}.md` === base)?.slug;
    if (slug) return `${slug}.html${tail}`;
  }
  return href;
}

function renderMarkdown(md, repoPath) {
  const ctx = { toc: [], ids: new Map(), text: [], paras: [], title: null, repoPath };
  const html = makeMarked(ctx).parse(md);
  // index everything readable, including tables, lists and code, for search
  ctx.text = [html.replace(/<(script|style)[^]*?<\/\1>/g, ' ').replace(/<[^>]+>/g, ' ')
    .replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&amp;/g, '&')];
  return { ...ctx, html };
}

// ---------------------------------------------------------------- layout
const ICON = {
  github: '<svg viewBox="0 0 16 16" width="18" height="18" aria-hidden="true"><path fill="currentColor" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z"/></svg>',
  search: '<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" d="M11 19a8 8 0 1 1 0-16 8 8 0 0 1 0 16zm10 2-4.35-4.35"/></svg>',
  menu: '<svg viewBox="0 0 24 24" width="20" height="20" aria-hidden="true"><path stroke="currentColor" stroke-width="2" stroke-linecap="round" d="M4 6h16M4 12h16M4 18h16"/></svg>'
};

function layout({ title, description, root, active, body, bodyClass = '', canonical }) {
  const fullTitle = title ? `${title} · ${config.title}` : `${config.title}: ${config.tagline}`;
  const link = (href, label, key) => `<a href="${root}${href}"${active === key ? ' aria-current="page"' : ''}>${label}</a>`;
  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${esc(fullTitle)}</title>
<meta name="description" content="${esc(description || config.description)}">
<meta name="theme-color" content="#070b12">
<meta property="og:type" content="website">
<meta property="og:title" content="${esc(fullTitle)}">
<meta property="og:description" content="${esc(description || config.description)}">
<meta property="og:image" content="${config.url}/assets/img/og-tulpa.jpg">
<meta name="twitter:card" content="summary_large_image">
${canonical ? `<link rel="canonical" href="${config.url}/${canonical}">` : ''}
<link rel="icon" type="image/svg+xml" href="${root}assets/img/hermisim-mark.svg">
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Space+Grotesk:wght@500;600;700&family=IBM+Plex+Sans:ital,wght@0,400;0,500;0,600;1,400&family=JetBrains+Mono:wght@400;500&display=swap" rel="stylesheet">
<link rel="stylesheet" href="${root}assets/css/site.css">
<script>window.SITE_ROOT=${JSON.stringify(root)};</script>
<script defer src="${root}assets/js/site.js"></script>
</head>
<body class="${bodyClass}">
<a class="skip" href="#main">Skip to content</a>
<header class="topbar">
  <div class="topbar-inner">
    <a class="brand" href="${root}index.html" aria-label="Tulpa and HermiSim home">
      <span class="brand-mark"><img src="${root}assets/img/hermisim-mark.svg" alt="" width="30" height="30"></span>
      <span class="brand-text">Tulpa<span class="brand-amp">&amp;</span>HermiSim</span>
    </a>
    <nav class="mainnav" aria-label="Main">
      ${link('tulpa.html', 'Tulpa', 'tulpa')}
      ${link('hermisim.html', 'HermiSim', 'hermisim')}
      ${link('docs/index.html', 'Docs', 'docs')}
      ${link('docs/examples.html', 'Examples', 'examples')}
    </nav>
    <div class="topbar-actions">
      <button class="search-btn" type="button" data-search-open>${ICON.search}<span>Search docs</span><kbd>/</kbd></button>
      <a class="icon-btn" href="${config.tulpa.repo}" target="_blank" rel="noopener" aria-label="TulpaLang on GitHub">${ICON.github}</a>
      <button class="icon-btn menu-btn" type="button" aria-label="Open menu" data-menu>${ICON.menu}</button>
    </div>
  </div>
</header>
<main id="main">
${body}
</main>
<footer class="footer">
  <div class="footer-inner">
    <div class="footer-brand">
      <img src="${root}assets/img/hermisim-mark.svg" alt="" width="36" height="36">
      <div><strong>Tulpa &amp; HermiSim</strong><br><span>${esc(config.tagline)}. A ${esc(config.org)} project.</span></div>
    </div>
    <div class="footer-cols">
      <div><h4>Tulpa</h4><a href="${root}tulpa.html">Overview</a><a href="${root}docs/language-guide.html">Language guide</a><a href="${root}docs/libraries.html">Libraries</a><a href="${config.tulpa.repo}/releases" target="_blank" rel="noopener">Releases</a></div>
      <div><h4>HermiSim</h4><a href="${root}hermisim.html">Overview</a><a href="${root}docs/hermisim.html">IDE tour</a><a href="${root}docs/morty.html">Morty project</a><a href="${config.hermisim.repo}/releases" target="_blank" rel="noopener">Download</a></div>
      <div><h4>Project</h4><a href="${config.tulpa.repo}" target="_blank" rel="noopener">TulpaLang on GitHub</a><a href="${config.hermisim.repo}" target="_blank" rel="noopener">HermiSim on GitHub</a><a href="${root}docs/roadmap.html">Roadmap</a><a href="${root}docs/contributing.html">Write docs</a></div>
    </div>
  </div>
  <div class="footer-legal">Both projects are released under the BSD 3-Clause license. © ${new Date().getFullYear()} ${esc(config.org)}.</div>
</footer>
<div class="search" hidden data-search>
  <div class="search-panel" role="dialog" aria-modal="true" aria-label="Search documentation">
    <div class="search-input">${ICON.search}<input type="search" placeholder="Search the docs: units, machine, cosim…" autocomplete="off" spellcheck="false"><kbd>Esc</kbd></div>
    <ul class="search-results" role="listbox"></ul>
  </div>
</div>
</body>
</html>
`;
}

// ---------------------------------------------------------------- pages (HTML fragments with {{...}} includes)
function renderFragment(src) {
  let meta = {};
  src = src.replace(/^<!--\s*(\{[^]*?\})\s*-->\s*/, (_, j) => { meta = JSON.parse(j); return ''; });
  src = src.replace(/\{\{snippet:([\w./-]+)(?:\|([^}]*))?\}\}/g, (_, file, title) => {
    const p = path.join(SRC, 'snippets', file);
    const lang = { rav: 'tulpa', sh: 'bash', txt: 'text', json: 'json' }[path.extname(file).slice(1)] || 'text';
    return codeBlock(read(p), lang, title);
  });
  src = src.replace(/\{\{(tulpa|hermisim)\.(\w+)\}\}/g, (_, a, b) => esc(config[a][b]));
  return { meta, html: src };
}

// ---------------------------------------------------------------- examples gallery
function buildExamples() {
  const dir = path.join(REPO, 'examples');
  const order = ['tour', 'warehouse_robot', 'units', 'verify', 'timing', 'device_control', 'robot_arm', 'sensor_i2c', 'fieldbus', 'tasks', 'modules', 'native', 'hello_world'];
  const names = fs.readdirSync(dir).filter((n) => fs.existsSync(path.join(dir, n, 'main.rav')))
    .sort((a, b) => ((order.indexOf(a) + 1) || 99) - ((order.indexOf(b) + 1) || 99));
  const ctx = { toc: [], text: [] };
  let html = `<p class="lead">Every program below lives in <a href="${config.tulpa.repo}/tree/main/examples" target="_blank" rel="noopener"><code>examples/</code></a> in the TulpaLang repository. The output shown is its <code>expected.txt</code>, which <code>cargo test</code> checks on every commit, under both the interpreter and the VM.</p>\n`;
  html += '<div class="ex-index">' + names.map((n) => `<a href="#${slugify(n)}">${n}</a>`).join('') + '</div>';
  for (const n of names) {
    const code = read(path.join(dir, n, 'main.rav'));
    const out = fs.existsSync(path.join(dir, n, 'expected.txt')) ? read(path.join(dir, n, 'expected.txt')) : '';
    const head = []; for (const l of code.split('\n')) { if (!l.startsWith('#')) break; head.push(l); }
    const lead = head.slice(0, 3).map((l) => l.replace(/^#\s?/, '').replace(/-{3,}/g, '').trim()).filter((l) => /[a-z]/i.test(l)).join(' ').trim();
    const id = slugify(n);
    ctx.toc.push({ depth: 2, id, text: n });
    ctx.text.push(n + ' ' + lead);
    html += `<section class="example" id="${id}"><h2><a class="anchor" href="#${id}" aria-hidden="true">#</a><code>${esc(n)}</code></h2>`;
    if (lead) html += `<p>${esc(lead)}</p>`;
    html += `<div class="ex-cmd"><code>tulpa examples/${esc(n)}/main.rav</code><a href="${config.tulpa.repo}/tree/main/examples/${esc(n)}" target="_blank" rel="noopener">View on GitHub →</a></div>`;
    html += `<div class="ex-pair">${codeBlock(code, 'tulpa', 'main.rav')}${out ? codeBlock(out, 'text', 'expected output') : ''}</div></section>`;
  }
  return { ...ctx, html, title: 'Example gallery', paras: ['Runnable Tulpa example programs from the TulpaLang repository, each with the expected output that cargo test checks on every commit.'] };
}

// ---------------------------------------------------------------- docs pages
function docSidebar(current, root) {
  return nav.map((s) => `<div class="side-group"><h3>${esc(s.section)}</h3><ul>` +
    s.pages.map((p) => `<li><a href="${p.slug}.html"${p.slug === current ? ' aria-current="page"' : ''}>${esc(p.title)}</a></li>`).join('') +
    '</ul></div>').join('');
}

function docPage(page, rendered, idx) {
  const prev = docPages[idx - 1], next = docPages[idx + 1];
  const toc = rendered.toc.filter((t) => t.depth >= 2);
  const editHref = page.repo
    ? `${config.siteRepo}/edit/${config.editBranch}/${page.repo}`
    : page.generated ? `${config.siteRepo}/tree/${config.editBranch}/examples`
      : `${config.siteRepo}/edit/${config.editBranch}/${config.siteDir}/src/docs/${page.file}`;
  const upstreamNote = page.repo ? `<div class="mirror-note">Source: <a href="${config.siteRepo}/blob/${config.editBranch}/${page.repo}" target="_blank" rel="noopener"><code>${page.repo}</code></a></div>` : '';
  const body = `
<div class="docs">
  <aside class="docs-side" aria-label="Documentation">
    <div class="side-inner">${docSidebar(page.slug)}</div>
  </aside>
  <article class="docs-main">
    <div class="crumbs"><a href="index.html">Docs</a><span>/</span>${esc(page.section)}</div>
    <h1>${esc(rendered.title || page.title)}</h1>
    ${upstreamNote}
    <div class="prose">${rendered.html}</div>
    <div class="doc-foot">
      <a class="edit" href="${editHref}" target="_blank" rel="noopener">Edit this page on GitHub</a>
    </div>
    <nav class="pager" aria-label="Pagination">
      ${prev ? `<a class="prev" href="${prev.slug}.html"><span>Previous</span>${esc(prev.title)}</a>` : '<span></span>'}
      ${next ? `<a class="next" href="${next.slug}.html"><span>Next</span>${esc(next.title)}</a>` : '<span></span>'}
    </nav>
  </article>
  <aside class="docs-toc" aria-label="On this page">
    ${toc.length ? `<div class="toc-inner"><h3>On this page</h3><ul>${toc.map((t) => `<li class="d${t.depth}"><a href="#${t.id}">${esc(t.text)}</a></li>`).join('')}</ul></div>` : ''}
  </aside>
</div>`;
  return body;
}

// ---------------------------------------------------------------- build
export function build() {
  const t0 = Date.now();
  fs.rmSync(OUT, { recursive: true, force: true });
  fs.mkdirSync(OUT, { recursive: true });
  copyDir(path.join(SRC, 'assets'), path.join(OUT, 'assets'));
  writeOut('.nojekyll', '');

  const search = [];
  const sitemap = [];

  // top-level pages
  for (const f of fs.readdirSync(path.join(SRC, 'pages')).filter((f) => f.endsWith('.html'))) {
    const { meta, html } = renderFragment(read(path.join(SRC, 'pages', f)));
    // 404.html is served at arbitrary depths by GitHub Pages, so it needs absolute URLs
    const root = f === '404.html' ? config.url.replace(/\/?$/, '/') : '';
    writeOut(f, layout({ title: meta.title, description: meta.description, root, active: meta.active, body: html.replaceAll('{{root}}', root), bodyClass: meta.bodyClass || '', canonical: f === 'index.html' ? '' : f }));
    if (f !== '404.html') sitemap.push(f === 'index.html' ? '' : f);
    if (meta.search) search.push({ t: meta.title || config.title, s: 'Site', u: f, x: meta.search });
  }

  // docs
  docPages.forEach((page, i) => {
    let rendered;
    if (page.generated === 'examples') rendered = buildExamples();
    else if (page.repo) rendered = renderMarkdown(read(path.join(REPO, page.repo)), page.repo);
    else rendered = renderMarkdown(read(path.join(SRC, 'docs', page.file)));
    const root = '../';
    const desc = ((rendered.paras || rendered.text).find((t) => t.length > 60) || config.description).replace(/\s+/g, ' ').slice(0, 180);
    writeOut(`docs/${page.slug}.html`, layout({
      title: page.title, description: desc, root, active: page.generated === 'examples' ? 'examples' : 'docs',
      body: docPage(page, rendered, i), bodyClass: 'is-docs', canonical: `docs/${page.slug}.html`
    }));
    sitemap.push(`docs/${page.slug}.html`);
    search.push({ t: page.title, s: page.section, u: `docs/${page.slug}.html`, x: rendered.text.join(' ').replace(/\s+/g, ' ').slice(0, 30000) });
    for (const h of rendered.toc) search.push({ t: h.text, s: page.title, u: `docs/${page.slug}.html#${h.id}`, x: '', h: 1 });
  });

  writeOut('search-index.json', JSON.stringify(search));
  writeOut('sitemap.xml', `<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${sitemap.map((u) => `  <url><loc>${config.url}/${u}</loc></url>`).join('\n')}\n</urlset>\n`);
  writeOut('robots.txt', `User-agent: *\nAllow: /\nSitemap: ${config.url}/sitemap.xml\n`);
  console.log(`built ${sitemap.length} pages into _site/ in ${Date.now() - t0} ms`);
}

// ---------------------------------------------------------------- dev server
function serve(port = 8080) {
  const types = { '.html': 'text/html', '.css': 'text/css', '.js': 'text/javascript', '.json': 'application/json', '.svg': 'image/svg+xml', '.png': 'image/png', '.jpg': 'image/jpeg', '.webp': 'image/webp', '.xml': 'application/xml' };
  http.createServer((req, res) => {
    let p = decodeURIComponent(new URL(req.url, 'http://x').pathname);
    if (p.endsWith('/')) p += 'index.html';
    const f = path.join(OUT, path.normalize(p));
    if (!f.startsWith(OUT) || !fs.existsSync(f)) { res.writeHead(404, { 'content-type': 'text/html' }); return res.end(fs.existsSync(path.join(OUT, '404.html')) ? read(path.join(OUT, '404.html')) : 'not found'); }
    res.writeHead(200, { 'content-type': types[path.extname(f)] || 'application/octet-stream' });
    fs.createReadStream(f).pipe(res);
  }).listen(port, () => console.log(`serving _site/ on http://localhost:${port}`));
  let timer;
  const rebuild = () => { clearTimeout(timer); timer = setTimeout(() => { try { build(); } catch (e) { console.error(e); } }, 150); };
  for (const d of [SRC, path.join(REPO, 'docs'), path.join(REPO, 'examples'), path.join(REPO, 'editors')]) {
    if (fs.existsSync(d)) fs.watch(d, { recursive: true }, rebuild);
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  build();
  if (process.argv.includes('--serve')) serve(Number(process.env.PORT) || 8080);
}
