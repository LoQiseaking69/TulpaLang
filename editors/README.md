# Editor setup for `tulpa lsp`

Build or install `tulpa` so it's on your `PATH`, then point your editor's LSP client at `tulpa lsp`
for files ending in `.rav` or `.tlp`.

## Neovim (built-in LSP, 0.10+)

```lua
vim.filetype.add({ extension = { rav = "tulpa", tlp = "tulpa" } })
vim.api.nvim_create_autocmd("FileType", {
  pattern = "tulpa",
  callback = function(args)
    vim.lsp.start({ name = "tulpa", cmd = { "tulpa", "lsp" }, root_dir = vim.fs.root(args.buf, { ".git" }) })
  end,
})
```

## VS Code

Install the Tulpa extension. It provides syntax highlighting, the language server, and Run/Test/Verify/Build commands.

```bash
code --install-extension tulpa-lang-<version>.vsix      # from a release, or:
cd editors/vscode && npm install && npm run package && code --install-extension tulpa-lang-*.vsix
```

If `tulpa` isn't on your `PATH`, set `"tulpa.serverPath"`.

## Helix (`languages.toml`)

```toml
[[language]]
name = "tulpa"
scope = "source.tulpa"
file-types = ["rav", "tlp"]
language-servers = ["tulpa-lsp"]
comment-token = "#"

[language-server.tulpa-lsp]
command = "tulpa"
args = ["lsp"]
```
