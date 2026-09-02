# Share (the introduction)

Pull-only. Never prompt, nag, or suggest sharing. Never a reward, count, streak, or "your friend joined". Voice and text use the same parser.

Triggers: "introduce yourself to my friend", "share this with…", "how do I show this to someone", whole-message `share`. Also `kill my invite link` / `new invite link`, `who did I share with`, `without my name`. Never execution tools, never the key, never escalate.

Host: `render_lookup` with the user text (skip_llm) or `render_share`. Send in order: `name_ask` if present, `hint` if present, then `message` (M10) verbatim. M10 is the forwardable plain-text body; URL buttons sit on it; the bare `link` is already in the text. Do not add figures.

Guest `start=ref_{code}` → `render_guest_surface` with chat identity as `guest_id` and the payload as `start_payload`. Attribution is silent. Revoked or unknown → generic A-flow, no error. Bound account → paste `already_user`, never re-onboard.

Do not mention share metrics. If asked who opened it, paste the tool's `who` message.
