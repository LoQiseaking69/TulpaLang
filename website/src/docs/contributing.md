# Writing docs

This site lives in the [`website/`](https://github.com/LoQiseaking69/TulpaLang/tree/main/website)
folder of the TulpaLang repository and is generated from Markdown. Adding a page takes two steps
and no build tooling beyond Node.js.

## Add a page

1. Create `website/src/docs/my-page.md`. The first `# Heading` becomes the page title.
2. Register it in `website/src/docs/_nav.json` under the right section:

```json
{ "slug": "my-page", "file": "my-page.md", "title": "My page" }
```

3. Preview locally:

```bash
cd website
npm install
npm run dev        # builds to _site/ and serves http://localhost:8080, rebuilding on change
```

Pushing to `main` rebuilds and deploys the site through GitHub Actions.

## Markdown features

| Feature | How |
|---|---|
| Tulpa highlighting | Fence code with ` ```tulpa ` (also `rav`). `bash`, `json`, `python`, `rust`, `toml`, `yaml`, `xml`, `lua` and `powershell` are highlighted too. |
| Program output | Fence with ` ```text `. |
| Callouts | A blockquote starting `> [!NOTE]`, `> [!TIP]`, `> [!WARNING]` or `> [!IMPORTANT]`. |
| Links between pages | Link the Markdown file: `[CLI](cli.md)` or `[units](language-guide.md#units-of-measure)`. They're rewritten to `.html` at build time. |
| Images | Put them in `src/assets/img/` and link `../assets/img/...`. |
| Table of contents | Generated automatically from `##` and `###` headings. |

> [!TIP]
> Every page has an **Edit this page** link that opens the source file on GitHub.

## Pages read from the repository

The language guide, libraries, device integration and editor setup pages are rendered straight
from `docs/language_guide.md`, `docs/libraries.md`, `docs/device_integration.md` and
`editors/README.md`, and the example gallery from `examples/`. There's one copy of each: edit
the file in place and the site picks it up on the next build. Such pages are registered in
`_nav.json` with `"repo"` instead of `"file"`:

```json
{ "slug": "libraries", "repo": "docs/libraries.md", "title": "Libraries" }
```

## Landing-page snippets

The code on the landing pages comes from `website/src/snippets/`, each with its real output in a
`.out.txt` file. After changing the language, rebuild and refresh them:

```bash
cargo build --release
cd website && npm run snippets
```

## Style

- Lead with what the reader can do, then the details.
- Keep code examples runnable, and show real output in a `text` block.
- Use units in examples (`0.5[m/s]`, not `0.5`). It's the language's point.
