import { filePath, readJson, writeJson } from "./store.js";

function queuePath() {
  return filePath("outbound.json");
}

export function enqueue(message) {
  const data = readJson(queuePath(), { items: [] });
  const item = {
    id: `out-${Date.now()}-${Math.random().toString(16).slice(2, 8)}`,
    queued_at: Math.floor(Date.now() / 1000),
    ...message,
  };
  data.items.push(item);
  writeJson(queuePath(), data);
  return item;
}

export function drain(limit = 50) {
  const data = readJson(queuePath(), { items: [] });
  const items = data.items.splice(0, limit);
  writeJson(queuePath(), data);
  return items;
}

export function peek() {
  return readJson(queuePath(), { items: [] }).items || [];
}
