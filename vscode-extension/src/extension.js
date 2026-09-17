// VS Code / Cursor extension client. Cursor is a VS Code fork that loads
// the same extension package via the same API -- this file, and the rest
// of this extension, needs no Cursor-specific branch or build.
const vscode = require("vscode");
const { LanguageClient, TransportKind } = require("vscode-languageclient/node");

let client;

function activate(context) {
  const serverPath = vscode.workspace.getConfiguration("annealscript").get("serverPath") || "annealscript-lsp";

  const serverOptions = {
    command: serverPath,
    transport: TransportKind.stdio,
  };

  const clientOptions = {
    documentSelector: [{ scheme: "file", language: "annealscript" }],
  };

  client = new LanguageClient("annealscript", "AnnealScript Language Server", serverOptions, clientOptions);
  client.start();

  context.subscriptions.push(
    vscode.commands.registerCommand("annealscript.runProfile", (uri) =>
      client.sendRequest("workspace/executeCommand", {
        command: "annealscript.runProfile",
        arguments: [uri.toString()],
      })
    )
  );
}

function deactivate() {
  return client ? client.stop() : undefined;
}

module.exports = { activate, deactivate };
