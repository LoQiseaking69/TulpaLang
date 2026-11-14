# Tulpa website & documentation

This folder is the source of the Tulpa and HermiSim website, published with GitHub Pages at
**https://loqiseaking69.github.io/TulpaLang/**. It holds the landing pages for
[Tulpa](../README.md) and [HermiSim](https://github.com/LoQiseaking69/HermiSim-DynamicsViewer)
and the documentation site.

It's a small static-site generator (`build.mjs`, under 400 lines) with two dependencies,
`marked` and `highlight.js`. The output is plain HTML, CSS and a little JavaScript, with no
framework and no client-side rendering.

## Working on the site

```bash
cd website
npm install
npm run dev      # http://localhost:8080, rebuilds on changes under website/src, docs/, examples/, editors/
npm run check    # build + verify every internal link and #anchor
```

## Publishing

`.github/workflows/pages.yml` builds, link-checks and deploys the site whenever `main` changes
`website/`, `docs/`, `examples/` or `editors/README.md`. You can also run it by hand from the
Actions tab. GitHub Pages must be set to deploy from Actions: **Settings ▸ Pages ▸ Source ▸
GitHub Actions**.

Links are relative throughout, so the site works under `/TulpaLang/` and on a custom domain.
If the address changes, update `url` in `site.config.json`, which is used for canonical URLs,
the sitemap and the 404 page.

## How the docs are assembled

There's a single copy of every page. Nothing is duplicated between the repo and the site.

| Site page | Source |
|---|---|
| Language guide, Libraries, Device integration | `docs/*.md` in the repo root |
| Editor setup | `editors/README.md` |
| Example gallery | `examples/*/main.rav` + `expected.txt` |
| Everything else in the docs | `website/src/docs/*.md` |
| Landing pages | `website/src/pages/*.html` |

The sidebar order lives in `website/src/docs/_nav.json`. Site-only pages use `"file"`, and pages
read from the repo use `"repo"`:

```json
{ "slug": "cli", "file": "cli.md", "title": "CLI reference" },
{ "slug": "libraries", "repo": "docs/libraries.md", "title": "Libraries" }
```

Relative links inside repo docs (`device_integration.md`, `../examples/...`) are rewritten to
site pages or GitHub URLs at build time. The **Writing docs** page
([`src/docs/contributing.md`](src/docs/contributing.md)) covers callouts, Tulpa code fences and
images.

## Layout

```text
website/
  site.config.json        versions, repo URLs, site URL
  build.mjs               generator (+ dev server with --serve)
  src/pages/              index.html, tulpa.html, hermisim.html, 404.html (HTML fragments)
  src/docs/               site-only docs pages + _nav.json
  src/snippets/           runnable .rav snippets for the landing pages, with their real output
  src/assets/             css, js, images and HermiSim screenshots
  scripts/tulpa-hljs.mjs  Tulpa grammar for highlight.js
  scripts/check-links.mjs internal link / anchor checker (run in CI)
  scripts/refresh-snippets.mjs  re-runs the snippets against target/release/tulpa
```

## Keeping the examples honest

The output under every landing-page snippet is real. After a language change:

```bash
cargo build --release
cd website && npm run snippets   # rewrites src/snippets/*.out.txt
```

`cosim.out.txt` and `morty.out.txt` come from HermiSim runs with MuJoCo and are updated by
hand.

## Page fragments

Files in `src/pages/` are HTML bodies with a JSON header comment:

```html
<!--{"title": "HermiSim", "active": "hermisim", "description": "…", "search": "keywords"}-->
```

| Include | Expands to |
|---|---|
| `{{snippet:units.rav}}` | Highlighted code block from `src/snippets/` (`.rav`, `.txt`, `.sh`, `.json`) |
| `{{snippet:units.out.txt\|$ tulpa units.rav}}` | The same, with a custom title |
| `{{tulpa.version}}`, `{{hermisim.repo}}`, … | Values from `site.config.json` |
