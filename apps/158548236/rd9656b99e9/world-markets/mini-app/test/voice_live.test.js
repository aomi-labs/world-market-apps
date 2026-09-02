import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const require = createRequire(import.meta.url);
const {
  shouldPaintInterim,
  foldStreamTranscript,
  preferHeardTranscript,
  liveCaptionIsCommand,
  snapshotPcm,
  encodeWavFromPcm,
  pickHoldAudio,
  expectedPcmWavBytes,
  floatToInt16,
  declaredStreamRate,
  resampleForStream,
  enqueueStreamPcm,
  pointInVoiceHit,
  MAX_STREAM_QUEUE,
  SLIDE_CANCEL_PAD_PX,
  PCM_FLUSH_FRAMES,
  PCM_FLUSH_MS,
  HOLD_TAIL_SILENCE_MS,
  padHoldPcm,
  pcmRms,
  isSpeechFrame,
  holdHadSpeech,
  speechHeardRecently,
  liveConfidenceOk,
  CHIRP_MS,
  CHIRP_TAIL_MS,
  SPEECH_RMS_FLOOR,
  SPEECH_RECENT_MS,
  LIVE_CONFIDENCE_MIN,
} = require("../static/voice_live.js");

test("does not paint empty or tiny interims", () => {
  assert.equal(shouldPaintInterim("", "", false), false);
  assert.equal(shouldPaintInterim("  ", "", false), false);
  assert.equal(shouldPaintInterim("hi", "", false), false);
  assert.equal(shouldPaintInterim("51", "", false), false);
  assert.equal(shouldPaintInterim("buy", "", false), true);
  assert.equal(shouldPaintInterim("buy ether", "", false), true);
});

test("final captions paint even when short", () => {
  assert.equal(shouldPaintInterim("hi", "", true), true);
  assert.equal(shouldPaintInterim("", "buy ether", true), false);
});

test("a placeholder or shorter final does not replace the caption already on screen", () => {
  assert.equal(shouldPaintInterim("you", "buy 5 ETH", true), false);
  assert.equal(shouldPaintInterim("thank you", "buy five eth", true), false);
  assert.equal(shouldPaintInterim("buy 5 ETH", "buy 5 ETH", true), true);
  const wiped = foldStreamTranscript("", "you", true, "buy 5 ETH");
  assert.equal(wiped.display, "buy 5 ETH");
  assert.equal(preferHeardTranscript("you", "buy 5 ETH"), "buy 5 ETH");
});

test("shorter interim does not replace a longer caption", () => {
  assert.equal(shouldPaintInterim("buy", "buy fifty dollars of ether", false), false);
  assert.equal(shouldPaintInterim("buy fifty dollars of ether", "buy", false), true);
});

test("foldStreamTranscript concatenates finals so a later span cannot drop the start", () => {
  let committed = "";
  let display = "";
  let step = foldStreamTranscript(committed, "what do you think about", false, display);
  assert.equal(step.display, "what do you think about");
  assert.equal(step.committed, "");
  committed = step.committed;
  display = step.display;
  step = foldStreamTranscript(committed, "what do you think about", true, display);
  assert.equal(step.committed, "what do you think about");
  committed = step.committed;
  display = step.display;
  step = foldStreamTranscript(committed, "compute futures", false, display);
  assert.equal(step.display, "what do you think about compute futures");
  assert.equal(step.committed, "what do you think about");
  committed = step.committed;
  display = step.display;
  step = foldStreamTranscript(committed, "compute futures", true, display);
  assert.equal(step.display, "what do you think about compute futures");
  assert.equal(step.committed, "what do you think about compute futures");
});

test("foldStreamTranscript keeps already-shown words when the first final is only a prefix", () => {
  const step = foldStreamTranscript(
    "",
    "what do you think about",
    true,
    "what do you think about compute futures",
  );
  assert.equal(step.committed, "what do you think about");
  assert.equal(step.display, "what do you think about compute futures");
});

test("foldStreamTranscript keeps a cumulative final that already includes the prefix", () => {
  const step = foldStreamTranscript(
    "what do you think about",
    "what do you think about compute futures",
    true,
  );
  assert.equal(step.display, "what do you think about compute futures");
});

test("hold-to-talk posts the full press-to-release recording, not the live caption", () => {
  const here = dirname(fileURLToPath(import.meta.url));
  const src = readFileSync(join(here, "../static/app.js"), "utf8");
  assert.match(src, /waitPcmFlushFrames/);
  assert.match(src, /snapshotLivePcm/);
  assert.match(src, /discardVoiceStream/);
  assert.match(src, /pickHoldAudio/);
  assert.match(src, /padHoldPcm/);
  assert.match(src, /stopRecorderBlob/);
  assert.match(src, /PCM_FLUSH_FRAMES/);
  assert.match(src, /live_text/);
  assert.doesNotMatch(src, /releaseHeard/);
  assert.doesNotMatch(src, /finalized:\s*usedStream/);
  assert.doesNotMatch(src, /msg\.is_final && voiceStreamOnFinal/);
  assert.doesNotMatch(src, /done\(String\(msg\.text/);
});

test("pickHoldAudio uses the complete MediaRecorder blob when PCM is truncated", () => {
  const wav = { size: 1000, type: "audio/wav" };
  const webm = { size: 8000, type: "audio/webm" };
  const expect = expectedPcmWavBytes(48000, 2);
  assert.ok(expect > 100000);
  assert.equal(pickHoldAudio(wav, webm, 48000, 2), webm);
  const seventy = { size: Math.floor(expect * 0.7), type: "audio/wav" };
  assert.equal(pickHoldAudio(seventy, webm, 48000, 2), webm);
  const full = { size: expect, type: "audio/wav" };
  assert.equal(pickHoldAudio(full, webm, 48000, 2), full);
  assert.equal(pickHoldAudio(full, null, 48000, 2), full);
});

test("slide-off requires leaving the button, not a lift inside it", () => {
  assert.equal(pointInVoiceHit(150, 150, 75, 75, 0, 0, 0), true);
  assert.equal(pointInVoiceHit(150, 150, 160, 75, 0, 0, 0), false);
  assert.equal(pointInVoiceHit(150, 150, 160, 75, 0, 0, SLIDE_CANCEL_PAD_PX), true);
  assert.equal(pointInVoiceHit(150, 150, 230, 75, 0, 0, SLIDE_CANCEL_PAD_PX), false);
});

test("release keeps a short tail so the last word is in the clip without extra hold", () => {
  assert.ok(PCM_FLUSH_FRAMES >= 6);
  assert.ok(PCM_FLUSH_MS >= 300 && PCM_FLUSH_MS <= 500);
  assert.ok(HOLD_TAIL_SILENCE_MS >= 300 && HOLD_TAIL_SILENCE_MS <= 500);
});

test("padHoldPcm appends trailing silence without mutating the hold", () => {
  const src = [new Float32Array([0.5, -0.25])];
  const padded = padHoldPcm(src, 8000, 100);
  assert.equal(src.length, 1);
  assert.equal(src[0][0], 0.5);
  const samples = padded.reduce((n, row) => n + row.length, 0);
  assert.equal(samples, 2 + 800);
  assert.equal(padded[0][0], 0.5);
  assert.equal(padded[padded.length - 1][0], 0);
  assert.equal(padHoldPcm([], 48000, 400).length, 0);
});

test("preferHeardTranscript keeps a longer finalized live sentence", () => {
  assert.equal(
    preferHeardTranscript("buy 51", "buy fifty dollars of ether"),
    "buy fifty dollars of ether",
  );
  assert.equal(preferHeardTranscript("Hi", "sell all sol"), "sell all sol");
  assert.equal(
    preferHeardTranscript("buy fifty dollars of ETH", "by 15 of it"),
    "buy fifty dollars of ETH",
  );
  assert.equal(
    preferHeardTranscript("buy fifty ETH", "buy fifty dollars of ether"),
    "buy fifty dollars of ether",
  );
  assert.equal(preferHeardTranscript("sell all sol", ""), "sell all sol");
  assert.equal(
    preferHeardTranscript("buy fifty dollars worth of ease", "buy 550 worth of ETH."),
    "buy 550 worth of ETH.",
  );
});

test("live buy captions count as commands even when clip STT is garbage", () => {
  assert.equal(liveCaptionIsCommand("buy 550 worth of ETH."), true);
  assert.equal(liveCaptionIsCommand("about 5 dollars worth"), true);
  assert.equal(liveCaptionIsCommand("By twenty dollars"), true);
  assert.equal(liveCaptionIsCommand("I have 50 ETH"), true);
  assert.equal(liveCaptionIsCommand("well 50 ETH"), true);
  assert.equal(liveCaptionIsCommand("cell fifty dollars"), true);
  assert.equal(liveCaptionIsCommand("A $20 worth"), true);
  assert.equal(liveCaptionIsCommand("A $20 worth"), true);
  assert.equal(liveCaptionIsCommand("how much is ETH"), false);
  assert.equal(liveCaptionIsCommand(""), false);
});

test("snapshotPcm copies buffers so teardown cannot wipe the utterance", () => {
  const src = [new Float32Array([0.5, -0.25])];
  const snap = snapshotPcm(src, 2, 48000);
  src[0][0] = 0;
  assert.equal(snap.chunks[0][0], 0.5);
  assert.equal(snap.samples, 2);
  assert.equal(snap.rate, 48000);
});

test("encodeWavFromPcm writes pcm16 mono matching the capture rate", async () => {
  const wav = encodeWavFromPcm([new Float32Array(8)], 44100);
  assert.ok(wav);
  assert.equal(wav.type, "audio/wav");
  const buf = Buffer.from(await wav.arrayBuffer());
  assert.equal(buf.toString("ascii", 0, 4), "RIFF");
  assert.equal(buf.toString("ascii", 8, 12), "WAVE");
  assert.equal(buf.readUInt16LE(20), 1, "WAVE_FORMAT_PCM, not mu-law");
  assert.equal(buf.readUInt16LE(22), 1, "mono");
  assert.equal(buf.readUInt32LE(24), 44100);
  assert.equal(buf.readUInt32LE(28), 44100 * 2);
  assert.equal(buf.readUInt16LE(32), 2);
  assert.equal(buf.readUInt16LE(34), 16);
  assert.equal(encodeWavFromPcm([], 48000), null);
});

test("declaredStreamRate keeps 44.1k and 16k; resamples only outside 8–48k", () => {
  assert.equal(declaredStreamRate(44100), 44100);
  assert.equal(declaredStreamRate(48000), 48000);
  assert.equal(declaredStreamRate(16000), 16000);
  assert.equal(declaredStreamRate(8000), 8000);
  assert.equal(declaredStreamRate(96000), 48000);
  assert.equal(declaredStreamRate(4000), 8000);
});

test("resampleForStream is a no-op when rates already match", () => {
  const src = new Float32Array([0.1, 0.2, 0.3]);
  const out = resampleForStream(src, 44100, 44100);
  assert.equal(out.length, 3);
  assert.equal(out[1], src[1]);
  const down = resampleForStream(new Float32Array(16), 96000, 48000);
  assert.equal(down.length, 8);
});

test("hold-to-talk plays an on chirp, then arms capture after the tail", () => {
  const here = dirname(fileURLToPath(import.meta.url));
  const src = readFileSync(join(here, "../static/app.js"), "utf8");
  assert.match(src, /function playVoiceOnSound\(/);
  assert.match(src, /function cueVoiceReady\(/);
  assert.match(src, /function markVoiceReady\(/);
  assert.match(src, /function startHoldRecorder\(/);
  const cue = src.slice(src.indexOf("function cueVoiceReady("), src.indexOf("function markVoiceReady("));
  assert.match(cue, /playVoiceOnSound\(/);
  assert.match(cue, /CHIRP_TAIL_MS/);
  assert.match(cue, /armAfterChirp/);
  const play = src.slice(src.indexOf("function playVoiceOnSound("), src.indexOf("function mapVoiceLevel("));
  assert.doesNotMatch(play, /!voiceReady/);
  const ready = src.slice(src.indexOf("function markVoiceReady("), src.indexOf("function beginListening("));
  assert.match(ready, /startHoldRecorder\(\)/);
  assert.match(ready, /voiceHoldArmed = true/);
  assert.doesNotMatch(ready, /playVoiceOnSound\(\)/);
  const begin = src.slice(src.indexOf("function beginListening("), src.indexOf("function bindHoldRecorder("));
  assert.doesNotMatch(begin, /playVoiceOnSound\(\)/);
  assert.doesNotMatch(begin, /voiceRecorder\.start/);
  assert.match(begin, /cueVoiceReady/);
});

test("hold-to-talk stream URL uses the declared capture rate", () => {
  const here = dirname(fileURLToPath(import.meta.url));
  const src = readFileSync(join(here, "../static/app.js"), "utf8");
  assert.match(src, /declaredStreamRate/);
  assert.match(src, /liveStreamRate/);
  assert.match(src, /resampleForStream/);
  assert.match(src, /sampleRate:\s*48000/);
  assert.match(src, /createScriptProcessor\(2048,\s*1,\s*1\)/);
});

test("floatToInt16 is little-endian pcm for the stream", () => {
  const samples = floatToInt16(new Float32Array([0, 1, -1]));
  assert.equal(samples.length, 3);
  assert.equal(samples[0], 0);
  assert.equal(samples[1], 0x7fff);
  assert.equal(samples[2], -0x8000);
});

test("stream queue keeps the start of the hold and does not grow without bound", () => {
  const queue = [];
  for (let i = 0; i < MAX_STREAM_QUEUE + 5; i++) {
    enqueueStreamPcm(queue, i);
  }
  assert.equal(queue.length, MAX_STREAM_QUEUE);
  assert.equal(queue[0], 0);
  assert.equal(queue[MAX_STREAM_QUEUE - 1], MAX_STREAM_QUEUE - 1);
});

test("pcmRms distinguishes hush from a spoken-scale frame", () => {
  assert.ok(pcmRms(new Float32Array(2048)) < SPEECH_RMS_FLOOR);
  assert.equal(isSpeechFrame(new Float32Array(2048)), false);
  const spoken = new Float32Array(2048);
  spoken.fill(0.2);
  assert.ok(pcmRms(spoken) > SPEECH_RMS_FLOOR);
  assert.equal(isSpeechFrame(spoken), true);
  assert.equal(holdHadSpeech([new Float32Array(64), spoken]), true);
  assert.equal(holdHadSpeech([new Float32Array(64), new Float32Array(64)]), false);
  const quietSpeech = new Float32Array(2048);
  quietSpeech.fill(0.0032);
  assert.equal(isSpeechFrame(quietSpeech), true);
});

test("speechHeardRecently and liveConfidenceOk gate hallucinations", () => {
  assert.equal(speechHeardRecently(0, 1000), false);
  assert.equal(speechHeardRecently(1000, 1200), true);
  assert.equal(speechHeardRecently(1000, 1000 + SPEECH_RECENT_MS + 1), false);
  assert.equal(liveConfidenceOk(undefined), true);
  assert.equal(liveConfidenceOk(0), true);
  assert.equal(liveConfidenceOk(0.4), false);
  assert.equal(liveConfidenceOk(LIVE_CONFIDENCE_MIN), true);
  assert.equal(liveConfidenceOk(0.9), true);
});

test("quiet holds skip STT instead of posting hush", () => {
  const here = dirname(fileURLToPath(import.meta.url));
  const src = readFileSync(join(here, "../static/app.js"), "utf8");
  assert.match(src, /holdHadSpeech/);
  assert.match(src, /voiceHadSpeech/);
  const commit = src.slice(src.indexOf("async function commitVoice("), src.indexOf("function landVoiceDraft("));
  assert.match(commit, /if \(!hadSpeech && pcm\.samples > 0\)/);
  assert.match(commit, /voiceEmpty/);
  assert.match(commit, /teardownVoice\(\)/);
  assert.ok(commit.indexOf("if (!hadSpeech)") < commit.indexOf("submitVoiceBlob"));
});

test("hold-to-talk requests speech-oriented capture", () => {
  const here = dirname(fileURLToPath(import.meta.url));
  const src = readFileSync(join(here, "../static/app.js"), "utf8");
  assert.match(src, /echoCancellation:\s*true/);
  assert.match(src, /noiseSuppression:\s*true/);
  assert.match(src, /autoGainControl:\s*true/);
  assert.match(src, /voiceIsolation:\s*true/);
  assert.match(src, /contentHint\s*=\s*["']speech["']/);
});

test("hold-to-talk shows a wait meter and honest copy while gates run", () => {
  const here = dirname(fileURLToPath(import.meta.url));
  const app = readFileSync(join(here, "../static/app.js"), "utf8");
  const copy = readFileSync(join(here, "../static/copy.js"), "utf8");
  assert.match(app, /id="voiceWaitMeter"/);
  assert.match(app, /function paintVoiceWait\(/);
  assert.match(app, /meter queue-fill voice-wait/);
  assert.match(copy, /connecting:\s*"connecting/);
  assert.match(copy, /ready:\s*"ready/);
  assert.match(copy, /speakNow:\s*"speak now/);
  assert.match(copy, /opening:\s*"OPENING"/);
  assert.ok(CHIRP_MS >= 150 && CHIRP_MS <= 220);
  assert.ok(CHIRP_TAIL_MS >= 20 && CHIRP_TAIL_MS <= 50);
});

test("voice questions skip the ledger and open the answer sheet", () => {
  const here = dirname(fileURLToPath(import.meta.url));
  const src = readFileSync(join(here, "../static/app.js"), "utf8");
  const copy = readFileSync(join(here, "../static/copy.js"), "utf8");
  const submit = src.slice(
    src.indexOf("async function submitVoiceBlob("),
    src.indexOf("async function commitVoice("),
  );
  assert.match(copy, /heard_with_ref:\s*'heard: "\{text\}" — the \{ref\}'/);
  assert.match(copy, /working:\s*"checking the engine — a moment"/);
  assert.match(copy, /footer_thread:/);
  assert.match(copy, /handoff_command:/);
  assert.match(copy, /handoff_escalation:/);
  assert.match(src, /function openAnswerSheet\(/);
  assert.match(src, /function closeAnswerSheet\(/);
  assert.match(src, /\/api\/v1\/mini-app\/answer\//);
  assert.match(submit, /context_ref: currentContextRefFromView\(\)/);
  assert.match(submit, /out && out\.question/);
  assert.match(submit, /utterance_kind/);
  assert.match(submit, /kind === "cant"/);
  assert.match(submit, /dropOptimistic\(correlation_id\)/);
  assert.match(submit, /openAnswerSheet\(/);
  assert.doesNotMatch(submit, /state\.view = "sent"/);
  assert.match(submit, /\/api\/v1\/mini-app\/compose/);
  assert.match(submit, /landVoiceDraft\(\s*heard,\s*correlation_id/);
  assert.match(submit, /liveLooksLikeCommand/);
  assert.match(submit, /landMisheard\(correlation_id\)/);
  const wallAt = submit.search(/kind === "cant"/);
  const liveOverrideAt = submit.search(/isQuestion && liveIsCommand/);
  const mixedAt = submit.search(/else if \(isMixed\) \{/);
  const questionAt = submit.search(/else if \(isQuestion\) \{/);
  const draftAt = submit.search(/landVoiceDraft\(\s*heard,\s*correlation_id/);
  assert.ok(wallAt >= 0 && liveOverrideAt >= 0 && mixedAt >= 0 && questionAt >= 0 && draftAt >= 0);
  assert.ok(wallAt < liveOverrideAt, "unclear live-command override precedes the question split");
  assert.ok(liveOverrideAt < mixedAt, "live command captions must not take the question sheet path");
  assert.ok(mixedAt < questionAt, "mixed utterances split before the question-only sheet");
  assert.ok(questionAt < draftAt, "true questions must not fall through to landVoiceDraft");
  const questionBranch = submit.slice(questionAt, draftAt);
  assert.match(questionBranch, /dropOptimistic\(correlation_id\)/);
  assert.match(questionBranch, /openAnswerSheet\(/);
  assert.doesNotMatch(questionBranch, /landQueuedTask/);
  assert.doesNotMatch(questionBranch, /landVoiceDraft/);
  assert.doesNotMatch(questionBranch, /openThreadLink/);
  const sheet = src.slice(
    src.indexOf("function openAnswerSheet("),
    src.indexOf("function closeAnswerSheet("),
  );
  const bind = src.slice(
    src.indexOf("function bindAnswerSheet("),
    src.indexOf("async function sendAnswerClarify("),
  );
  assert.equal(
    bind.split("openThreadLink(").length - 1,
    1,
    "question path may redirect to the thread only on escalation handoff",
  );
  assert.match(bind, /answerOpenThread/);
  assert.match(sheet, /function openAnswerSheet\(/);
  assert.match(src, /function scheduleAnswerWorkingPaint\(/);
  assert.match(src, /function syncAnswerHandoffFromLedger\(/);
  assert.match(src, /function applyInSheetHeard\(/);
  assert.match(src, /ex.status === "handoff_command"/);
  const wallBranch = submit.slice(wallAt, liveOverrideAt);
  assert.match(wallBranch, /state.sheet === "answer"/);
  assert.match(wallBranch, /applyInSheetHeard/);
});

test("unclear voice reuses the client row as a grey misheard card", () => {
  const here = dirname(fileURLToPath(import.meta.url));
  const src = readFileSync(join(here, "../static/app.js"), "utf8");
  const copy = readFileSync(join(here, "../static/copy.js"), "utf8");
  const submit = src.slice(
    src.indexOf("async function submitVoiceBlob("),
    src.indexOf("async function commitVoice("),
  );
  const outcome = src.slice(
    src.indexOf("function applyHeardOutcome("),
    src.indexOf("async function sendNearMatchChoice("),
  );
  assert.match(copy, /misheard:\s*"Misheard, try again"/);
  assert.match(src, /function landMisheard\(/);
  assert.match(src, /status === "misheard"/);
  assert.match(submit, /applyHeardOutcome\([^)]*correlation_id\)/);
  assert.match(submit, /landVoiceDraft\(\s*heard,\s*correlation_id/);
  assert.doesNotMatch(submit, /applyHeardOutcome\([^)]*\bcid\b/);
  assert.match(outcome, /kind === "unclear"[\s\S]*landMisheard\(correlation_id\)/);
});

test("silent PCM is kept alive, not streamed, until speech energy", () => {
  const here = dirname(fileURLToPath(import.meta.url));
  const src = readFileSync(join(here, "../static/app.js"), "utf8");
  assert.match(src, /function sendStreamKeepAlive\(/);
  assert.match(src, /type:\s*["']KeepAlive["']/);
  assert.match(src, /speechHeardRecently/);
  assert.match(src, /liveConfidenceOk/);
});

test("DECISIONS records speech-oriented capture, not the Deepgram demo defaults", () => {
  const here = dirname(fileURLToPath(import.meta.url));
  const src = readFileSync(join(here, "../DECISIONS.md"), "utf8");
  assert.match(src, /voiceIsolation/);
  assert.match(src, /~40ms, not a pause/);
  assert.doesNotMatch(src, /same as the Deepgram demo/);
});

test("toasts are a corner box sized from the record button, green or red", () => {
  const here = dirname(fileURLToPath(import.meta.url));
  const css = readFileSync(join(here, "../static/styles.css"), "utf8");
  const src = readFileSync(join(here, "../static/app.js"), "utf8");
  assert.match(css, /--voice-hero:\s*150px/);
  assert.match(css, /\.toast\s*\{[\s\S]*?width:\s*var\(--voice-hero\)/);
  assert.match(css, /\.toast\s*\{[\s\S]*?height:\s*calc\(var\(--voice-hero\)\s*\/\s*2\)/);
  assert.match(css, /\.toast\s*\{[\s\S]*?right:\s*12px/);
  assert.match(css, /\.toast\s*\{[\s\S]*?bottom:\s*calc\(12px \+ var\(--safe-bot\)\)/);
  assert.match(css, /\.toast\s*\{[\s\S]*?max-width:\s*calc\(100vw - 24px\)/);
  assert.match(css, /\.toast\.good\s*\{[\s\S]*?background:\s*var\(--pos\)/);
  assert.match(css, /\.toast\.bad\s*\{[\s\S]*?background:\s*var\(--neg\)/);
  assert.match(src, /function toastTone\(/);
  assert.match(src, /class="toast \$\{kind\}"/);
  assert.match(src, /showToast\(C\.toasts\.voiceEmpty,\s*"bad"\)/);
  assert.match(src, /showToast\(C\.toasts\.voiceFailed,\s*"bad"\)/);
  const submitFail = src.slice(
    src.indexOf("async function submitVoiceBlob("),
    src.indexOf("async function commitVoice("),
  );
  const catchAt = submitFail.lastIndexOf("} catch");
  assert.ok(catchAt > 0);
  const catchBlock = submitFail.slice(catchAt);
  assert.doesNotMatch(catchBlock, /dropOptimistic/);
  assert.match(catchBlock, /landQueuedTask\(liveCaption/);
  assert.match(src, /showToast\(C\.draftRow\.misheard[\s\S]*?"bad"\)/);
  assert.match(src, /showToast\(fillCopy\(C\.toasts\.voiceHeard,\s*\{ text \}\),\s*"good"\)/);
  const land = src.slice(
    src.indexOf("function landMisheard("),
    src.indexOf("function applyHeardOutcome("),
  );
  assert.match(land, /showToast\([\s\S]*?"bad"\)/);
});

test("optimistic voice rows only yield to a newly appeared ledger sentence", () => {
  const here = dirname(fileURLToPath(import.meta.url));
  const src = readFileSync(join(here, "../static/app.js"), "utf8");
  const refresh = src.slice(
    src.indexOf("async function refreshLedger("),
    src.indexOf("function patchLiveClock("),
  );
  assert.match(refresh, /!priorById\[led\.instruction_id\]/);
  assert.match(refresh, /!priorById\[row\.instruction_id\]/);
});
