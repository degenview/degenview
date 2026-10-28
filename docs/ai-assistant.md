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
| Tool use in `service/llm` (wire formats, `stream_turn`); nothing calls a tool yet | Done (agent PR 1) |
| Agent loop, private draft, `read_script`/`compile`/`edit_script`/`write_script`, tool steps in the panel | Done (agent PR 2) |
| `profile` tool (profile the draft on frozen bars) | Done (agent PR 3) |
| Plan mode (`submit_plan`, plan card) | Planned (agent PR 4) |
| Compile-and-repair loop, runtime-error context, selection edits | Superseded by the agent loop |

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

## Tool use

`ChatRequest::tools` (`ToolSpec`: name, description, JSON Schema) lets a model call functions, and
`LlmProvider::stream_turn` returns a `TurnReply { text, calls, stop }` instead of text alone. A provider
that only chats keeps the default `stream_turn`, which wraps `stream_chat` and never calls a tool.
`HttpProvider` implements both over the same decoders. The agent loop (next section) is the caller.

- **Messages carry blocks.** `ChatMessage` keeps its `content` string and gains `tool_calls` (on an
  assistant message) and `tool_results` (on the user message that answers them). A message with neither
  is built exactly as before, and a request with no tools has no `tools` field.
- **Anthropic**: `tools[].input_schema`; a `tool_use` block arrives as `content_block_start` (its `input`
  is a placeholder, ignored) and `input_json_delta` fragments, joined per block index; results are
  `tool_result` blocks first in a user message, with `is_error` set when the call failed.
- **OpenAI and compatible hosts**: `tools[].function.parameters`; `delta.tool_calls` fragments joined by
  `index` (a host that sends each whole call at index 0 with a new id is split, and a missing id is
  made up as `call_N`); results are one `role: "tool"` message each, with a failure marked "Error: "
  in its text because the format has no flag. Arguments are a string of JSON there.
- **Arguments** are parsed when the turn ends. Empty means `{}` (a tool with no arguments streams
  nothing); text that is not a JSON object leaves `ToolCall::malformed` set so the caller answers the
  model with an error instead of failing the request.
- **A turn of calls only is a reply** (`run_turn`); `run_stream` still rejects a reply with no text. A
  turn that stopped at the token limit returns `Stop::MaxTokens` and **no calls**: their arguments are
  cut off and cannot be trusted.
- Anthropic extended thinking is off. With it on, the thinking blocks of a tool turn would have to be
  sent back unchanged, which nothing here does.
- Tests: hand-written frames for both formats (fragments, two calls, no arguments, bad arguments,
  placeholder input), the request bodies, and a real `HttpProvider` against a loopback server that
  streams a call. **Not verified**: a real OpenAI or Anthropic tool round trip; the frame shapes come
  from the providers' documentation.

## The agent loop

A request is no longer one reply. `ai_agent::run` asks the model, runs the tools it calls, gives it the
answers and asks again until it stops calling. It runs on the request's own thread, so `ai.rs` (state),
`ai_host` (builds the request) and `ai_panel` (draws it) are as before.

- **The assistant never touches the editor.** `ai_send` copies the open script into a `Draft`
  (`ai_tools`); every tool reads or changes that copy. When the request ends the draft is compared with
  the script the request started from and becomes the usual `Proposal`: summary card, review sheet, Apply
  as one undo step, refused if the editor no longer holds `proposal.base`. "Nothing changes until Apply"
  and "Ask never edits" are unchanged.
- **Tools** (`ToolBox::new(mode, …)` offers and runs only the mode's own): `read_script(start?, end?)` (numbered
  lines of the draft, 400 at a time), `compile()` (the draft's diagnostics, as the compiler words them),
  and in Edit `edit_script(old, new)` and `write_script(text)`. A call to a tool the mode does not have is
  an error *result* (the model is told), never an edit, so Ask cannot write even if the model tries.
- **`edit_script` is an exact, unique match.** No match, or more than one (it lists the lines), is an
  error the model reads and corrects. A change answers with whether the draft still compiles
  ("Now: 1 error." and the first problems), so the model does not need a `compile` call to find out. The
  draft is held with `\n` endings whatever the file uses, because the model writes `\n`;
  `match_line_endings` puts the file's own back when the proposal is built.
- **Limits are in the loop, not the prompt**: 12 calls to the model (`MAX_ROUNDS`; each resends the whole
  conversation, so this bounds cost), a call repeated with the same arguments three times ends the
  request, a tool answer is cut at 8,000 characters, `cancel` is checked between rounds and calls.
- **Work is not thrown away.** When a request ends without finishing (the step limit, a repeated call, a
  provider failure part-way) or the user presses Stop, and the draft differs from the script, it is
  offered as a proposal with a `note` ("Incomplete. It used all 12 steps without finishing.", "Stopped
  before it finished.") shown in orange on the card. It still goes through review. A failure still puts the
  message back in the box and shows the provider's error; Stop still restores the message.
- **A server that refuses tools** (HTTP 400/404/422 on the first request, as a local server that does not
  know them may answer) is asked once more without tools and with the whole-script prompt
  (`ai_prompt::system_prompt`), and the user is told. That path (a fenced script in the reply) is the
  one that existed before and is unchanged. A request that fails a second time is the error.
- **Two prompts.** `agent_system_prompt` (draft, tools, finish with one to three sentences and do not
  paste the script) and `system_prompt` (the fallback). Both share `push_reference`, the list of what
  this interpreter does not implement.
- **What the panel shows.** Between a message and its reply, one line per tool call: a spinner while it
  runs, then a check or a cross and what it came to ("Edited lines 12–14 · No problems"), which
  opens to what the model was told; the text the model said before calling is shown muted. This trail
  (`Assistant::steps`) is for the latest exchange only and is not saved. **History keeps the words**, so
  the prompt tells the model to say what it found in its final reply.
- Ask has the reading tools (it can look, not change), `profile` included. That makes an Ask turn able to
  take several rounds; "why is this slow?" is the reason.

### The `profile` tool

`ai_profile::ProfileTool`, offered in every mode. It runs the **draft** (not the editor's text) through
the engine's profiler (`pine::profile::ProfileSession`, [Pine profiler](pine-profiler.md)) on a frozen
copy of the preview chart's bars, taken when the request was sent (`preview.debug_feed`, so at most 500
candles, with the preview's inputs, symbol and timeframe). It is a session of its own on its own thread:
no alerts, orders, preview or chart are touched. It is **not** the UI's `Profiler`, so it never draws on
the editor's Profile column or tab and a profile the user made is not disturbed.

- **What the model is told**: totals, the ten hottest lines by their own time (share, own time, time with
  everything under it, executions, the line's source), the hottest block and function, time in no line,
  time waiting for `request.security()` data, and how to read it (shares are meaningful, absolute times
  are of that run, per statement not per expression, the clock's resolution, the profiler's cost). The
  words are `ProfileMarks`' (`duration`, `percent`, `count`), so the model and the user see the same
  figures.
- **A second profile in the same request is compared with the first**: total time and measured
  executions, with the change as a percentage. Executions are exact and do not vary; times do, and the
  text says not to read a small difference in time as a result. Only complete profiles are compared.
- **Refused as an answer, not a failure**: the draft has errors (they are listed), it is a library, the
  preview had no bars, the script fails when it runs (a failure has no profile, as in the Profile tab), or
  the request was stopped (the run is asked to stop; if it will not within three seconds the tool gives
  up).
- **What leaves the Mac**: the words above, including trimmed source lines (the script is sent anyway).
  The bars never do.
- **`security_data`** is the app's `KlineSecurityDataProvider`, as the Profile tab uses: a script that
  reads another symbol may fetch its candles over the network, and that time is reported as waiting.
- Tests: a real profile of a small loop (executions exact: 1,500 for a 50-iteration body on 30 bars),
  the comparison, a failing script, no bars, a library, a stopped request, and the whole window in Ask.
- Tests: the tools and the loop against the scripted provider (call, answer, final; several calls in a
  turn; a failed call; Ask refused; step cap; repeat guard; cancel; refused tools; no tools = one request),
  the draft becoming a proposal (CRLF kept, nothing changed, failure and Stop and step limit keep the work),
  and the whole window (an edit through tools, Apply, the trail drawing at a small size).
  Pictures: `script_manager::ai_gallery` (scenes "tools", "tools-incomplete").
- **Not verified**: any of this against a real model. The scripted provider returns what the wire
  formats say a model returns; how well real models use these tools, how many rounds a typical request
  takes, and what it costs have not been measured.

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

## Ask and Edit modes

- A new conversation starts in **Ask**, and so does opening another script. The Ask | Edit switch
  sits in the composer; it is locked while a request runs or a proposal waits, so Ask can never sit
  beside an Apply button. Start over keeps the mode.
- **Ask never changes the script, in code and not only in the prompt.** `Assistant::finish` makes no
  `Proposal` for an Ask request even if the model returns a whole script, and `ai_accept` refuses
  unless the mode is Edit. A script the model writes in Ask shows as a code block with a Copy
  button. `ai_prompt::system_prompt(mode)` tells the model it cannot change the script, to answer
  or give a short numbered plan, and to keep any snippet under about 15 lines.
- Each assistant `Turn` keeps the mode it was made in. History resent to the model drops an Edit
  reply's script (the current one is resent) but keeps an Ask reply whole.
- An Ask reply with two or more numbered steps (`looks_like_plan`) gets an **Implement this plan**
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

## The panel, history and the model menu

- **Layout**: the header has the model chip (provider and model; it opens the model menu); user
  messages are right-aligned bubbles; replies are full width with Copy and Ask again under the last
  one; the composer holds the box, the Ask | Edit switch and Send in one frame; the line under it
  names what a request sends (`Sent to OpenAI · Ask never edits · Bollinger Squeeze · 42 lines ·
  No problems`). What leaves the Mac comes first, because a narrow panel cuts the end of the line.
- **Errors** are a card (`ui::error_notice`, shared by the panel, the model menu and Settings) with **Try again**
  (the message is back in the box), **Copy details** and a dismiss button. The card shows an
  `ErrorReport` (`service::llm::report`): a headline, up to 400 characters of the provider's message, and the
  request id. `LlmError::report()` builds it from the *structure* of the error (the variant, the HTTP status,
  the body's `error.message`, the `x-request-id` / `request-id` response header), never by reading the message's
  prose, so a provider can reword, translate or add errors and the screens still hold. Do not turn an `LlmError`
  into a string before it reaches a screen: `Notice::failure`, `ModelList::Failed` and `StatusMessage::failure`
  carry the report. The only exact-text rule is dropping a closing "(… request ID <id> …)" note when it contains
  the very id the header gave. Layout tests run every shape in `service::llm::sample` (short, empty, one huge
  word, many lines, non-ASCII, no id), not a pasted provider message. Copy gives the whole report. The model menu
  is fixed at `MENU_WIDTH` whatever the error says. Settings footers (`settings_window::footer_row`, also
  Alpaca, CoinMarketCap, Snapshots and Webhooks) lay the buttons out first and cut the status to one line with
  an ellipsis (hover shows all), so a long error can never push Save out of the bar; the AI page also shows the
  report as a card at the top.
- **Retry and Edit** work on the last exchange only (`Assistant::rewind_last`): Retry sends the message
  again in the mode it was asked in; Edit puts it in the box.
- **Model menu** (`ai_models`): the providers (one without a key opens Settings) and the current
  provider's models from its key's model list, fetched when the menu opens and kept for the session.
  OpenAI's list is filtered to chat models (embedding, speech, image and moderation ids are hidden); a
  filter box narrows the list, and typing a name the list does not show offers "Use …". The choice is
  saved to `ai.prefs`, which Settings reads. The Settings page re-reads its draft when the saved
  prefs change under it, and the host re-reads the prefs every second, so neither can put an old
  choice back. A text field inside a menu must not ask for infinite width, or the menu grows to fill the
  window.
- **Saved history** (`ai_history`): each script's conversation is saved in the local setting
  `ai.history` and restored when that script is opened, so it survives a restart. It is capped (30
  conversations by last use, 40 turns each, 40k characters a turn, 120k a conversation, 1M in all; the
  oldest turns go first and a conversation never starts with a reply) and never syncs. Writes are
  read-modify-write against the store, so a history cleared in Settings is not brought back by a copy in
  memory. A deleted script's conversation goes with it. Settings > AI Assistant has **Clear saved
  conversations**. Only text is stored: not proposals, not the review, not the mode.

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

**Not verified**: the model menu against real provider lists (the OpenAI filter is a heuristic; the
"Use …" row is the way out) and clicking in it; the review sheet in a real window (clicking rows and toggles, scrolling a very long
file, a very small window); clicking a link or the Copy button in a real window (the character-under-pointer
mapping for links is tested, the click itself is not); using the panel in a real window (typing, Enter to send, focus, scroll feel, the
Keychain prompt on Send); any request to the real OpenAI or Anthropic API (no keys were used), so the frame
shapes come from the providers' documentation rather than recorded traffic, and the suggested
default model names (`gpt-5`, `claude-sonnet-5-5`) may need updating; the Settings page in a real
window (layout, the eye button, the spinner); the Keychain path on a signed build.
