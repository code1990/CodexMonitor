import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";

const strict = process.argv.includes("--strict");

function runCommand(command, args) {
  try {
    return spawnSync(command, args, { stdio: "pipe", encoding: "utf8" });
  } catch {
    return null;
  }
}

function isExecutableFile(filePath) {
  try {
    const stat = fs.statSync(filePath);
    if (!stat.isFile()) return false;
    if (process.platform === "win32") return true;
    fs.accessSync(filePath, fs.constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

function hasCommand(command) {
  const pathValue = process.env.PATH;
  if (!pathValue) return false;

  const dirs = pathValue.split(path.delimiter).filter(Boolean);

  if (process.platform !== "win32") {
    return dirs.some((dir) => isExecutableFile(path.join(dir, command)));
  }

  const pathExtValue = process.env.PATHEXT ?? ".EXE;.CMD;.BAT;.COM";
  const exts = pathExtValue.split(";").filter(Boolean);
  const hasExtension = path.extname(command) !== "";

  for (const dir of dirs) {
    if (hasExtension) {
      if (isExecutableFile(path.join(dir, command))) return true;
      continue;
    }
    for (const ext of exts) {
      if (isExecutableFile(path.join(dir, `${command}${ext}`))) return true;
    }
  }

  return false;
}

const missing = [];
if (!hasCommand("cmake")) missing.push("cmake");
if (process.platform === "win32" && !hasCommand("clang")) missing.push("llvm");
if (process.platform === "linux" && !hasCommand("pkg-config")) missing.push("pkg-config");

const issues = [];
if (process.platform === "linux" && hasCommand("pkg-config")) {
  const glibCheck = runCommand("pkg-config", ["--exists", "glib-2.0 >= 2.70"]);
  if (!glibCheck || glibCheck.status !== 0) {
    const versionCheck = runCommand("pkg-config", ["--modversion", "glib-2.0"]);
    const version = versionCheck?.status === 0 ? versionCheck.stdout.trim() : null;
    issues.push(
      version
        ? `glib-2.0 >= 2.70 required for Tauri builds/tests (found ${version})`
        : "glib-2.0 >= 2.70 required for Tauri builds/tests",
    );
  }
}

if (missing.length === 0 && issues.length === 0) {
  console.log("Doctor: OK");
  process.exit(0);
}

if (missing.length > 0) {
  console.log(`Doctor: missing dependencies: ${missing.join(" ")}`);
}
for (const issue of issues) {
  console.log(`Doctor: ${issue}`);
}

switch (process.platform) {
  case "darwin":
    console.log("Install: brew install cmake");
    break;
  case "linux":
    console.log(`Detected Linux: ${os.release()}`);
    console.log(
      "Ubuntu/Debian: sudo apt-get install cmake pkg-config libgtk-3-dev libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libasound2-dev libglib2.0-dev",
    );
    console.log(
      "Fedora: sudo dnf install cmake pkgconf-pkg-config gtk3-devel webkit2gtk4.1-devel libappindicator-gtk3 librsvg2-devel alsa-lib-devel glib2-devel",
    );
    console.log(
      "Arch: sudo pacman -S cmake pkgconf gtk3 webkit2gtk-4.1 libappindicator-gtk3 librsvg alsa-lib glib2",
    );
    console.log("Or use the reproducible shell: nix develop");
    break;
  case "win32":
    console.log("Install: choco install cmake llvm");
    console.log("Or download from: https://cmake.org/download/");
    console.log("If bindgen fails, set LIBCLANG_PATH to your LLVM bin directory.");
    break;
  default:
    console.log("Install CMake from: https://cmake.org/download/");
    break;
}

process.exit(strict ? 1 : 0);
