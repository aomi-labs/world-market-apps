import { mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { homedir } from "node:os";

export function dataDir() {
  if (process.env.WORLD_BRAIN_DIR) return process.env.WORLD_BRAIN_DIR;
  const xdg = process.env.XDG_DATA_HOME;
  const root = xdg
    ? join(xdg, "aomi/world-markets/brain")
    : join(homedir(), ".local/share/aomi/world-markets/brain");
  return root;
}

export function readJson(path, fallback) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch {
    return fallback;
  }
}

export function writeJson(path, value) {
  mkdirSync(dirname(path), { recursive: true });
  const tmp = `${path}.${process.pid}.tmp`;
  writeFileSync(tmp, `${JSON.stringify(value, null, 2)}\n`);
  renameSync(tmp, path);
}

export function filePath(...parts) {
  return join(dataDir(), ...parts);
}
