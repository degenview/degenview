# AI assistant

Lets a user prompt an LLM from the Script Manager to change the open Pine script, using **their
own API key**. This page records what exists, why it is shaped this way, and what is not verified.

## Status

| Part | State |
| --- | --- |
| `service/llm`: providers, streaming, errors | Done (phase 1) |
| `service/keychain_ai_keys`: per-provider key store | Done (phase 1) |
| Settings ▸ AI Assistant: provider, model, key, Test Connection | Done (phase 1) |
| Assistant panel under the editor, proposal diff, accept as one undo step | Done (phase 2) |
| Compile-and-repair loop, runtime-error context, selection edits | Not started (phase 3) |

## Why a user-supplied key

- OpenAI and Anthropic API access is key-based. Neither offers "sign in and bill my account" OAuth
  to a third-party desktop app, and consumer subscription logins (ChatGPT Plus, Claude Pro) are not
  for third-party apps. Do not build on them.
- OpenRouter offers an OAuth PKCE flow that returns a key. That could be a later "one-click" option;
  it needs a loopback listener, which only the MCP server has today.
- A hosted proxy would be the smoothest UX but means running a backend and billing. Out of scope.
- There is no DegenView server in the path: requests go from the user's Mac to the provider.

## Providers

`ProviderKind` is `OpenAi`, `Anthropic` and `Compatible` (any host that copies OpenAI's chat
completions API: OpenRouter, Ollama, LM Studio, Groq, ...).

- OpenAI: `POST {base}/chat/completions`, `Authorization: Bearer`, `max_completion_tokens` (its
  reasoning models reject `max_tokens`).
- Compatible: the same, with `max_tokens`, and the key is optional (a local server has none).
- Anthropic: `POST {base}/v1/messages`, `x-api-key`, `anthropic-version: 2023-06-01`, top-level
  `system`.
- Both stream server-sent events. `sse::read_events` frames them; `openai::OpenAiDecoder` and
  `anthropic::AnthropicDecoder` turn frames into text. A stream that ends without the provider's
  closing frame is an error, never a half reply that looks complete.
- Test Connection calls the models list (`GET /models`, `GET /v1/models`), which spends no tokens.

## Rules

- **Keys are Keychain-only** (`AiKeyStore`, service `es.nico.degenview.ai`, one account per
  provider). Never put one in `ai.prefs`, a saved layout, a log, or an error message.
  `HttpProvider` replaces the key with `[key]` in every error it returns.
- `DEGENVIEW_OPENAI_KEY`, `DEGENVIEW_ANTHROPIC_KEY` and `DEGENVIEW_LLM_KEY` stand in when nothing is
  saved (developer convenience, like the other providers' variables).
- **Off macOS** the Keychain write fails; the store still keeps the key for the session and the page
  says it is not saved for later.
- **`ai.prefs`** (provider, models, compatible server URL) is a local `setting`; it is in the
  never-sync test in `sync::policy` because it names the user's own account and server.
- **Base URLs** must be `https://`, or `http://` to `localhost`, `127.0.0.1` or `::1`
  (`validate_base_url`). A user name in the URL is rejected so `http://localhost@evil.example`
  cannot pass.
- **No redirects** are followed, so a redirect cannot carry the key to another host.
- Blocking `ureq` on a worker thread, no async runtime (core guide). `cancel` is an `AtomicBool`
  checked between SSE lines; a connection that stalls mid-line ends at the read timeout, so the UI
  should also detach from the worker when the user presses Stop.
- The first time the assistant is used, the user must be told the script and prompt are sent to the
  provider (the Settings page already says so).

## The panel (phase 2)

- `ai_panel` (view), `ai_host` (`impl ScriptManager`), `ai` (state), `ai_prompt` (pure text rules).
  The toolbar's sparkles button toggles it (`prefs.ai_open`, `ai_height`); it sits under the editor
  and the debugger's panel, in an outer split.
- **One conversation per open script.** Opening another script, or Start over, forgets it and stops
  any request. Only the last six turns are resent, with earlier scripts replaced by a marker; the
  current script and its compiler diagnostics are always in the last message.
- **The model returns the whole script** in one fenced block, never a patch. `ai_prompt::split_reply`
  picks the block that looks like a script (a `//@version` or a declaration), or the only block.
  A block still open when the stream ends, or a `length` stop, is refused: a cut-off script is never
  offered. A script equal to the current one (ignoring line endings and trailing blank space) is
  "nothing to change".
- **Nothing changes until Apply.** A finished reply becomes a `Proposal` (base text, script, diff,
  and the error and warning counts from compiling it off-screen). `ai_accept` refuses if the editor
  no longer holds exactly `proposal.base`, so typing made while the model wrote is never overwritten.
  Apply goes through `editor::apply_script`, which shares `apply_undoable` with the change-marker
  revert: one undo step, caret on the first changed line. Never use `replace_source` for this; it
  leaves no undo entry. A proposal that does not compile can still be applied ("Apply anyway").
- A request runs on its own `std::thread` (not a `WorkQueue` lane, so a stalled connection never
  holds up the next request). Stop detaches at once and restores the typed message; the thread ends
  when its connection next yields. A failed request puts the message back in the box.
- The line endings of the user's file are kept (`match_line_endings`).
- Enter sends and Shift-Enter adds a line; Enter is left alone while an input method is composing.
- The panel says what it sends ("Sent to OpenAI: this script and your message") above the Send
  button, and a missing key shows a setup card with Open Settings.
- Pictures for a person to look at: `DEGENVIEW_GALLERY_DIR=… cargo test --lib
  script_manager::ai_gallery -- --ignored` (both themes, wide and narrow).

## Plan and Edit modes

- A new conversation starts in **Plan**, and so does opening another script. The Plan | Edit switch
  sits in the composer; it is locked while a request runs or a proposal waits, so Plan can never sit
  beside an Apply button. Start over keeps the mode.
- **Plan never changes the script, in code and not only in the prompt.** `Assistant::finish` makes no
  `Proposal` for a Plan request even if the model returns a whole script, and `ai_accept` refuses
  unless the mode is Edit. A script the model writes in Plan shows as a code block with a Copy
  button. `ai_prompt::system_prompt(mode)` tells the model it cannot change the script, to answer
  or give a short numbered plan, and to keep any snippet under about 15 lines.
- Each assistant `Turn` keeps the mode it was made in. History resent to the model drops an Edit
  reply's script (the current one is resent) but keeps a Plan reply whole.
- A Plan reply with two or more numbered steps (`looks_like_plan`) gets an **Implement this plan**
  button: it switches to Edit and sends `IMPLEMENT_MESSAGE`, with the plan already in history.
- The user's message shows from the moment it is sent (`Assistant::pending_request`), not only
  when the reply ends, and the transcript scrolls to the end when a request starts or finishes.

## The review sheet

A proposal is a whole new script, but it is reviewed as separate changes. The panel's card is a
summary (counts, whether it compiles, up to three changes) with **Review changes**, **Apply all** and
**Discard**. Review changes opens a large sheet (`ai_review_view`, drawn by `sheets_ui`):

- A list of changes on the left (hidden under about 760 pt of sheet width) with an accept box each,
  and the whole file's diff on the right: old and new line numbers, syntax colour, the changed words
  marked, and unchanged stretches folded (three lines of context; a fold must hide at least four).
- Each change can be accepted or rejected. **Apply N of M** applies only the accepted ones
  (`diff::merge_selected`) as one undo step through `editor::apply_script`, refused if the editor no
  longer holds `proposal.base`. The footer says whether *the current selection* compiles, recompiled on
  every toggle. With nothing selected Apply does nothing.
- Keys: ↑ ↓ or K J move between changes, Space toggles the current one, ⌘↩ applies, Esc closes (the
  proposal stays). The editor and composer give up the keyboard when the sheet opens, because they
  draw before it and would otherwise get those keys.
- `ai_review` is pure (rows, folds, `Review` state); `diff::word_ranges` marks words with
  `capture_diff_slices` on tokens (no new `similar` feature), skipped for lines over 300 characters and
  for lines that changed almost entirely.
- Dimming a rejected change must scale the tint's alpha (`gamma_multiply`), not replace it: the diff
  backgrounds are already translucent, and `theme::with_alpha` replaces alpha. This went wrong once,
  and only the light theme showed it.

## Reply rendering

`ui/markdown_view.rs` renders replies: paragraphs, headings, bullet and numbered items, quotes,
fenced code (pine code is syntax-coloured; each block has Copy) and inline bold, italic, `code`
and http(s) links. It is a small parser, not CommonMark: it never fails and never waits, so a fence
still open while streaming is an open code block, and a `**` with nothing to close it is two
stars. Underscores are never emphasis (`plot_color` stays `plot_color`). Only http(s) links are
made clickable.

## Verified and not

Verified by tests: SSE framing; both wire formats against hand-written frames; request bodies and
headers; a real `HttpProvider` against a loopback server (key header, streamed reply, 401 scrubbed,
redirect refused, model list); key store behaviour including the refusing-Keychain case; the form
rules on the Settings page.

Also verified headlessly: a scripted provider driving the whole window (reply to proposal, accept,
stale refusal, discard, switching scripts, a failed request, a missing key), and the applied script
as one undo step and redo in the editor.

**Not verified**: the review sheet in a real window (clicking rows and toggles, scrolling a very long
file, a very small window); clicking a link or the Copy button in a real window (the character-under-pointer
mapping for links is tested, the click itself is not); using the panel in a real window (typing, Enter to send, focus, scroll feel, the
Keychain prompt on Send); any request to the real OpenAI or Anthropic API (no keys were used), so the frame
shapes come from the providers' documentation rather than recorded traffic, and the suggested
default model names (`gpt-5`, `claude-sonnet-5-5`) may need updating; the Settings page in a real
window (layout, the eye button, the spinner); the Keychain path on a signed build.
