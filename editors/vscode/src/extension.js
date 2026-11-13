// Tulpa for VS Code: starts `tulpa lsp` and adds run/test/verify/build/graph commands.
const vscode = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

let client;
let terminal;

function exe() {
  return vscode.workspace.getConfiguration("tulpa").get("serverPath") || "tulpa";
}

function startClient(context) {
  const serverOptions = {
    run: { command: exe(), args: ["lsp"], transport: TransportKind.stdio },
    debug: { command: exe(), args: ["lsp"], transport: TransportKind.stdio },
  };
  const clientOptions = {
    documentSelector: [{ scheme: "file", language: "tulpa" }, { scheme: "untitled", language: "tulpa" }],
  };
  client = new LanguageClient("tulpa", "Tulpa Language Server", serverOptions, clientOptions);
  client.start().catch((err) => {
    vscode.window.showErrorMessage(
      `Tulpa: could not start '${exe()} lsp' (${err.message}). Install tulpa or set "tulpa.serverPath".`
    );
  });
  context.subscriptions.push({ dispose: () => client && client.stop() });
}

function quote(s) {
  return /[\s"']/.test(s) ? `"${s.replace(/"/g, '\\"')}"` : s;
}

async function runInTerminal(args) {
  const editor = vscode.window.activeTextEditor;
  if (!editor || editor.document.languageId !== "tulpa") {
    vscode.window.showWarningMessage("Open a Tulpa file (.rav / .tlp) first.");
    return;
  }
  if (editor.document.isDirty) await editor.document.save();
  const file = editor.document.fileName;
  if (!terminal || terminal.exitStatus !== undefined) terminal = vscode.window.createTerminal("Tulpa");
  terminal.show(true);
  terminal.sendText([exe(), ...args(file)].map(quote).join(" "));
}

function activate(context) {
  const cfg = () => vscode.workspace.getConfiguration("tulpa");
  const cmds = {
    "tulpa.run": () => runInTerminal((f) => [...cfg().get("runArgs", []), f]),
    "tulpa.test": () => runInTerminal((f) => ["test", f]),
    "tulpa.verify": () => runInTerminal((f) => ["verify", f]),
    "tulpa.build": () => runInTerminal((f) => ["build", f, "-o", f.replace(/\.(rav|tlp)$/, ".o")]),
    "tulpa.graph": () => runInTerminal((f) => ["graph", f]),
    "tulpa.restartServer": async () => {
      if (client) await client.stop();
      startClient(context);
    },
  };
  for (const [name, fn] of Object.entries(cmds)) {
    context.subscriptions.push(vscode.commands.registerCommand(name, fn));
  }
  // Commands work even when the language server can't start (e.g. tulpa not installed yet).
  try {
    startClient(context);
  } catch (err) {
    vscode.window.showErrorMessage(`Tulpa: language server failed to start: ${err.message}`);
  }
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
