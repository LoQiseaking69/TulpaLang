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

Install a generic LSP client extension, for example "Generic LSP Client" or `glspc`, and add this to your settings:

```json
{
  "files.associations": { "*.rav": "tulpa", "*.tlp": "tulpa" },
  "glspc.languageId": "tulpa",
  "glspc.serverCommand": "tulpa",
  "glspc.serverCommandArguments": ["lsp"]
}
```

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
