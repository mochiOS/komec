import * as fs from "node:fs";
import * as path from "node:path";

import * as vscode from "vscode";

import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

export async function activate(
    context: vscode.ExtensionContext,
): Promise<void> {
    const outputChannel = vscode.window.createOutputChannel(
        "Kome Language Server",
        {
            log: true,
        },
    );

    context.subscriptions.push(outputChannel);

    const serverPath = resolveServerPath(context);

    const workspaceRoot =
        vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;

    const serverOptions: ServerOptions = {
        command: serverPath,
        transport: TransportKind.stdio,
        options: {
            cwd: workspaceRoot ?? context.extensionPath,
            env: process.env,
        },
    };

    const clientOptions: LanguageClientOptions = {
        documentSelector: [
            {
                scheme: "file",
                language: "kome",
            },
            {
                scheme: "untitled",
                language: "kome",
            },
        ],
        outputChannel,
    };

    client = new LanguageClient(
        "komeLanguageServer",
        "Kome Language Server",
        serverOptions,
        clientOptions,
    );

    try {
        await client.start();

        outputChannel.info(
            `Started Kome Language Server: ${serverPath}`,
        );
    } catch (error: unknown) {
        const message = error instanceof Error
            ? error.message
            : String(error);

        outputChannel.error(
            `Failed to start Kome Language Server: ${message}`,
        );

        outputChannel.show(true);

        await vscode.window.showErrorMessage(
            `Failed to start Kome Language Server: ${message}`,
        );
    }
}

export async function deactivate(): Promise<void> {
    if (client === undefined) {
        return;
    }

    await client.stop();
    client = undefined;
}

function resolveServerPath(
    context: vscode.ExtensionContext,
): string {
    const configuredPath = vscode.workspace
        .getConfiguration("kome")
        .get<string>("server.path")
        ?.trim();

    if (configuredPath) {
        if (!fs.existsSync(configuredPath)) {
            throw new Error(
                `kome.server.path does not exist: ${configuredPath}`,
            );
        }

        return configuredPath;
    }

    const executableName = process.platform === "win32"
        ? "kome-lsp.exe"
        : "kome-lsp";

    const developmentPath = path.resolve(
        context.extensionPath,
        "..",
        "..",
        "target",
        "debug",
        executableName,
    );

    if (fs.existsSync(developmentPath)) {
        return developmentPath;
    }

    return executableName;
}