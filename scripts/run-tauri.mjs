import { spawnSync } from "node:child_process";
import path from "node:path";

const args = process.argv.slice(2);

if (args.length === 0) {
  console.error("Usage: node scripts/run-tauri.mjs <tauri-args...>");
  process.exit(1);
}

const tauriArgs =
  process.platform === "win32"
    ? [...args, "--config", "src-tauri/tauri.windows.conf.json"]
    : args;

if (
  process.platform === "win32" &&
  args.includes("build") &&
  !process.env.TAURI_SIGNING_PRIVATE_KEY
) {
  tauriArgs.push("--config", "src-tauri/tauri.windows.local.conf.json");
}

const result =
  process.platform === "win32"
    ? spawnSync(
        "cmd.exe",
        [
          "/d",
          "/s",
          "/c",
          path.join(process.cwd(), "node_modules", ".bin", "tauri.cmd"),
          ...tauriArgs,
        ],
        {
          stdio: "inherit",
          shell: false,
        },
      )
    : spawnSync(path.join(process.cwd(), "node_modules", ".bin", "tauri"), tauriArgs, {
        stdio: "inherit",
        shell: false,
      });

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}

process.exit(result.status ?? 0);
