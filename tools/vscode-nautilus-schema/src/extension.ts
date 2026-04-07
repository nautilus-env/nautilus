import * as fs from "fs";
import * as https from "https";
import * as os from "os";
import * as path from "path";
import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";

const GITHUB_REPO = "y0gm4/nautilus";
const BIN_NAME = process.platform === "win32" ? "nautilus-lsp.exe" : "nautilus-lsp";

let client: LanguageClient | undefined;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  let serverPath: string;

  try {
    serverPath = await resolveServerPath(context);
  } catch (err) {
    vscode.window.showErrorMessage(
      `nautilus-lsp: could not resolve binary — ${err}. ` +
        `Set "nautilus.lspPath" in your settings or add nautilus-lsp to PATH.`
    );
    return;
  }

  const serverOptions: ServerOptions = {
    command: serverPath,
    transport: TransportKind.stdio,
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "nautilus" }],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher("**/*.nautilus"),
    },
  };

  client = new LanguageClient(
    "nautilus-lsp",
    "Nautilus LSP",
    serverOptions,
    clientOptions
  );

  client.start();
  context.subscriptions.push(client);
}

export function deactivate(): Thenable<void> | undefined {
  return client?.stop();
}

// Path resolution

/**
 * Resolves the `nautilus-lsp` binary path.
 *
 * Search order:
 * 1. `nautilus.lspPath` VS Code setting (user-defined override).
 * 2. Dev build: `<repo-root>/target/debug/nautilus-lsp[.exe]`.
 * 3. Global storage cache (previously auto-downloaded binary).
 * 4. `nautilus-lsp[.exe]` on PATH.
 * 5. Auto-download from GitHub Releases -> cache in global storage.
 */
async function resolveServerPath(context: vscode.ExtensionContext): Promise<string> {
  const rawSetting = vscode.workspace
    .getConfiguration("nautilus")
    .get<string>("lspPath");
  if (rawSetting && rawSetting.trim() !== "") {
    const setting = rawSetting.trim().replace(/^~(?=$|\/|\\)/, os.homedir());
    if (fs.existsSync(setting)) {
      return setting;
    }
  }

  const devBuild = path.join(
    context.extensionPath,
    "..", "..",
    "target", "debug", BIN_NAME
  );
  if (fs.existsSync(devBuild)) {
    return devBuild;
  }

  const cachedPath = getCachedBinPath(context);
  if (fs.existsSync(cachedPath)) {
    return cachedPath;
  }

  if (isOnPath(BIN_NAME)) {
    return BIN_NAME;
  }

  return downloadLsp(context);
}

function getCachedBinPath(context: vscode.ExtensionContext): string {
  return path.join(context.globalStorageUri.fsPath, BIN_NAME);
}

/** Best-effort check whether a binary exists in any PATH directory. */
function isOnPath(bin: string): boolean {
  const pathEnv = process.env.PATH ?? "";
  const dirs = pathEnv.split(path.delimiter);
  return dirs.some((dir) => fs.existsSync(path.join(dir, bin)));
}

// Auto-download

/** Maps Node platform/arch to the Rust target triple used in release artifacts. */
function platformTarget(): string {
  const plat = process.platform;
  const arch = process.arch;

  if (plat === "linux" && arch === "x64")   { return "x86_64-unknown-linux-gnu"; }
  if (plat === "darwin" && arch === "x64")  { return "x86_64-apple-darwin"; }
  if (plat === "darwin" && arch === "arm64"){ return "aarch64-apple-darwin"; }
  if (plat === "win32"  && arch === "x64")  { return "x86_64-pc-windows-msvc"; }

  throw new Error(`Unsupported platform: ${plat}/${arch}`);
}

function releaseDownloadUrl(target: string): string {
  const asset =
    process.platform === "win32"
      ? `nautilus-lsp-${target}.exe`
      : `nautilus-lsp-${target}`;
  return `https://github.com/${GITHUB_REPO}/releases/latest/download/${asset}`;
}

async function downloadLsp(context: vscode.ExtensionContext): Promise<string> {
  const target = platformTarget();
  const url = releaseDownloadUrl(target);
  const dest = getCachedBinPath(context);

  const storageDir = context.globalStorageUri.fsPath;
  if (!fs.existsSync(storageDir)) {
    fs.mkdirSync(storageDir, { recursive: true });
  }

  return vscode.window.withProgress(
    {
      location: vscode.ProgressLocation.Notification,
      title: "Nautilus LSP",
      cancellable: false,
    },
    async (progress) => {
      progress.report({ message: "Downloading nautilus-lsp binary…" });
      await httpsDownload(url, dest);
      if (os.platform() !== "win32") {
        fs.chmodSync(dest, 0o755);
      }
      progress.report({ message: "nautilus-lsp downloaded." });
      return dest;
    }
  );
}

/** Downloads `url` (following HTTP redirects) to `dest`. Rejects on HTTP error. */
function httpsDownload(url: string, dest: string): Promise<void> {
  return new Promise((resolve, reject) => {
    const follow = (currentUrl: string) => {
      https
        .get(currentUrl, (res) => {
          if (
            res.statusCode &&
            res.statusCode >= 300 &&
            res.statusCode < 400 &&
            res.headers.location
          ) {
            follow(res.headers.location);
            return;
          }

          if (res.statusCode !== 200) {
            reject(
              new Error(
                `HTTP ${res.statusCode ?? "?"} downloading ${currentUrl}`
              )
            );
            return;
          }

          const file = fs.createWriteStream(dest);
          res.pipe(file);
          file.on("finish", () => file.close(() => resolve()));
          file.on("error", (err) => {
            fs.unlink(dest, () => undefined);
            reject(err);
          });
        })
        .on("error", reject);
    };

    follow(url);
  });
}
