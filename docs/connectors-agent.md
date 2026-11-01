# The agent connector

The agent connector is the coding agent that runs in the terminals of a space. Helm uses it to start agents, record and resume their sessions, restart them, name a space with Smart rename, and read the usage limits.

The built-in connector is `claude-code`. Another agent (Codex, Aider, Gemini CLI, and others) connects as an external program through the command form.

## Choose the connector

Set `connectors.agent` in projects.json:

```json
{ "connectors": { "agent": "claude-code" } }
{ "connectors": { "agent": "none" } }
{ "connectors": { "agent": { "command": "my-agent-connector --flag" } } }
```

The default is `claude-code`. The name `none` turns the agent kind off. Then `herdr-helm connectors` shows `{"name": "none", "capabilities": []}`, and the agent commands stop with a clear error. `info` still works, without sessions.

The settings of the connector are in `agent_settings`:

| Key | Type | Default | Use |
|---|---|---|---|
| `smart_command` | text | `claude -p --model haiku` | The command that Smart rename runs. Helm also reads the older `smart_rename.command`. |
| `usage` | true or false | false | Turns on `herdr-helm usage` for claude-code. It works only on macOS. |

## Capabilities

`describe` gives the capabilities. The commands and the mods use them to show only what works.

| Capability | Commands that need it |
|---|---|
| `start` | `spawn`, and the first agent of a new space |
| `sessions` | `sessions save`, the sessions in `info` |
| `resume` | `sessions resume` |
| `restart` | `restart` |
| `smart_rename` | `rename` without `--title` |
| `usage` | `usage` |

claude-code has `start`, `sessions`, `resume`, `restart` and `smart_rename`. It has `usage` only on macOS, and only when `agent_settings.usage` is true.

The `usage` command checks the `usage` capability first. Without it, the command stops with an error.

## The methods

The Rust trait is `Agent` in `src/connectors/agent/mod.rs`. Each method is also an op of the command form, with the same name.

| Op | Args | Result |
|---|---|---|
| `describe` | `{}` | `{"name", "label", "capabilities", "icon"}`. The `label` is the name for a person, for example "Claude Code". `icon` is the icon of the usage meter: `{"path", "color"}`, or a path alone. The path is an SVG file relative to the Herdr home, where Herdr loads icons from. `label` and `icon` are optional. |
| `start_argv` | `{"dirs": [...]}` | The command line that starts a new agent, as a list. `dirs` are extra folders that the agent may work in (the linked checkouts outside the space folder). |
| `programs` | `{}` | The program names of the agent, as a terminal's foreground process shows them, for example `["claude"]`. |
| `running_session` | `{"pid": 123}` | `{"session", "cwd"}` of the running agent process, or `null` when it has no session yet. |
| `resume_argv` | `{"argv": [...], "session": "id"}` | The command line `argv`, changed to resume the session. |
| `sessions_in` | `{"roots": ["/path", ...]}` | The sessions that started in these folders or under them, as a list of Session objects. Helm checks the folder again, so the connector may give more. |
| `sessions_by_id` | `{"ids": ["id", ...]}` | The sessions with these ids, in the same order. Leave out the ids that you do not know. |
| `smart_rename` | `{"prompt": "...", "schema": {...}}` | The answer of the agent, an object `{"title", "summary", "work"}` that matches the schema. |
| `usage` | `{}` | `{"at", "plan", "limits": [{"kind", "label", "short", "percent", "resetsAt", "seconds"}]}` |

The trait has one more method that is not an op: `smart_default`. It gives the Smart rename
command that the connector runs when `agent_settings.smart_command` is empty, or none. `settings`
shows it as `agent_settings.smart_default`, and Helm Settings shows it in the empty field.
claude-code gives `claude -p --model haiku`. A command connector gives none, so the field is
`null`.

A Session object has these keys. All are optional, except `session`:

```json
{
  "session": "the id",
  "title": "the title of the session, or null",
  "named": false,
  "cwd": "/the/start/folder",
  "prs": [{ "number": 12, "url": "https://github.com/org/web/pull/12", "repo": "web" }],
  "messages": 3,
  "prompt": "the first prompt",
  "last": "the last prompt",
  "at": 1793400000000,
  "sample": ["up to 12 prompts, from the first to the last"]
}
```

`named` is true when the person gave the title. `messages` is the number of prompts of the person. `at` is the time of the last activity, in ms. Helm shows only the sessions with one or more messages.

## The command form

Helm runs the command once for each call. The request is one JSON object on stdin:

```json
{ "kind": "agent", "op": "start_argv", "args": { "dirs": [] }, "settings": { "smart_command": "..." } }
```

`settings` is the `agent_settings` object of projects.json. Write the result as JSON on stdout. To fail, exit with a code that is not 0 and write the message on stderr. Helm shows all of stderr to the person. Each call must finish in 60 seconds, else Helm stops the program.

Helm asks `describe` once per process and keeps the reply. For an op that your agent cannot do, leave its capability out of `describe` and exit with an error.

## How claude-code works

claude-code reads the files that Claude Code keeps under `~/.claude`:

- `sessions/<pid>.json` holds the session of each running Claude process. This file is not documented. If its format changes, Helm finds no session and records nothing.
- `projects/<folder key>/<session id>.jsonl` is the transcript of each session. The folder key is the start folder, with each character that is not a letter or a digit changed to `-`.

`resume_argv` removes `--resume`, `-r`, `--continue` and `-c` (and the value of `--resume`) from the command line, and adds `--resume <id>`.

Smart rename runs `agent_settings.smart_command` in `state_dir/smart`, with the prompt on stdin. When the command is `claude`, Helm adds the flags for one answer: `-p`, `--output-format json`, `--no-session-persistence`, no tools, no MCP servers, no setting sources, and `--json-schema`. Another command gets the line "Answer with only the JSON object." at the end of the prompt. The agent must answer in 60 seconds.

`usage` reads the sign-in of Claude Code from the macOS keychain and asks the endpoint behind Claude Code's `/usage`. The token stays in a shell variable. curl reads the header from stdin, so the token is never in an argument list. Because it reads the keychain, `usage` is off until you set `agent_settings.usage` to true. It needs the macOS `security` program, so claude-code does not name `usage` on another OS.

The endpoint is not documented, so Helm cannot be sure of the shape of its reply. Helm reads two shapes: `{"limits": [{"kind", "percent", "resets_at"}]}`, and `{"five_hour": {"utilization", "resets_at"}, "seven_day": {...}, "seven_day_<model>": {...}}`. A field that is missing or unknown gives no limit, not an error. When the reply has neither shape, `limits` is an empty list. The tests use sample replies of these assumed shapes. They never call the endpoint.

## Files that Helm writes

Helm writes these files under `state_dir` (`<Herdr home>/helm/state`):

| File | Content |
|---|---|
| `sessions.json` | The session of each agent terminal, from `sessions save`. `sessions resume` uses it after a shutdown. |
| `space-titles.json` | The titles of the plain Herdr spaces, which have no space record. |
| `smart/` | The work folder of Smart rename, so that the agent leaves no files in a space. |
