# The code_host connector

The code_host connector is the place where the repos live: their PRs, reviews, and merge
queue. The commands `github`, `prs` and `pr` use it. The code is in
`src/connectors/code_host/` (the connectors) and `src/commands/code_host.rs` (the flow that
all code hosts share).

## Choose a connector

Set `connectors.code_host` in projects.json:

| Value | Connector |
| --- | --- |
| `"github"` (default) | github.com and GitHub Enterprise, through the `gh` CLI |
| `"none"` | No code host. `connectors` shows `{"name": "none", "capabilities": []}`. `prs` and `pr` stop with an error. `github` gives the repos with `"pr": null`. |
| `{"command": "<program>"}` | An external program that speaks the JSON contract below |

## Settings

The settings are in `code_host_settings`. The github connector reads these keys. For each
key that `code_host_settings` does not have, it reads the older `github` block.

| Key | Default | Use |
| --- | --- | --- |
| `host` | `"github.com"` | The GitHub host, for example a GitHub Enterprise host |
| `org` | none | The org of the `prs` command and of the PR links. `prs` and `pr` need it. |
| `refresh_minutes` | `5` | How often the mods read `prs` again |
| `merge_method` | the first that the repo allows of `squash`, `merge`, `rebase` | The merge method of `pr enqueue` on a repo without a merge queue |
| `stacks` | `false` | Ask GitHub for the stack of each PR (`stack`, `stackEntry`). The public GraphQL schema does not have these fields, so they are off by default. When the server does not know them, Helm asks again without them. Without `stacks`, each PR has `"stack": null`. |

The github connector serves the host from the settings, github.com, every `*.ghe.com` host,
and every host in the hosts.yml file of gh.

Each `gh` call must finish in 60 seconds. Then Helm stops `gh`, and the command fails with a
message that names `gh`.

## Capabilities

The github connector has these capabilities:

- `branch_prs`: the PR of the branch of a checkout (`github` command)
- `org_prs`: the open PRs of the org in sections (`prs` command)
- `review`: an agent reviews a PR (`pr review`)
- `fix`: an agent prepares a fix of a PR (`pr fix`)
- `merge_queue`: merge a PR when it is ready (`pr enqueue`)

Each command checks its capability first. See the table in [connectors.md](connectors.md).

## The command form

An external connector gets one JSON request on stdin and writes one JSON result on stdout:

```json
{"kind": "code_host", "op": "<op>", "args": {...}, "settings": {...code_host_settings}}
```

A non-zero exit is an error, and all of stderr is its message. Each call must finish in 60
seconds. Helm asks `describe` once per process. Each method of the `CodeHost` trait is an op
with the same name.

| Op | Args | Result |
| --- | --- | --- |
| `describe` | `{}` | `{"name": "...", "capabilities": ["..."]}` |
| `host_of` | `{"remote_url"}` | The host name of the connector, for example `"github"`, or `null` for a URL that it does not serve |
| `org_repo` | `{"remote_url"}` | The repo name in lowercase when the remote is a repo of the configured org, else `null` |
| `branch_pr` | `{"path", "branch", "upstream", "head"}` | `{"pr": {...} or null, "not_hosted": bool, "error": "..." or null}` |
| `org_prs` | `{}` | `{"viewer", "host", "org", "refresh_minutes", "prs": [PR], "rate": {...}, "warning"}` |
| `pr_info` | `{"repo", "number"}` | `{"number", "title", "url", "headRefName", "baseRefName", "author": {"login"}, "isCrossRepository"}`. `isCrossRepository` is `true` for a PR from a fork. Helm does not make a space for such a PR yet. |
| `pr_state` | `{"repo", "number"}` | `"OPEN"`, `"MERGED"`, `"CLOSED"`, or `null` |
| `branch_prs` | `{"repo", "branch"}` | `[{"number", "title", "url", "state", "author": {"login"}, "headRefName"}]` in any state |
| `pr_ref` | `{"url"}` | `{"repo", "number"}` of a PR web URL of this host, or `null` |
| `pr_at` | `{"url"}` | The PR at the URL in the `branch_prs` form plus `"repo"`, or `null` |
| `enqueue` | `{"repo", "number"}` | `{"message": "..."}` |
| `agent_prompt` | `{"action", "repo", "number", "title", "url", "head", "base", "path", "notes", "reasons"}` | The first message of the review or fix agent, as a string |

### The PR of a checkout (`branch_pr`)

The `pr` object has the keys that the helm-github mod reads: `number`, `title`, `state`,
`url`, `isDraft`, `reviewDecision`, `checks` (`"failing"`, `"pending"`, `"passing"` or
`null`), and `check_count`. The `github` command writes `"not_github": true` for
`not_hosted`, and `"pr_error"` for `error`. The mods know these keys.

### A PR of the org (`org_prs`)

Each PR has these keys: `key` (`"<repo>#<number>"`), `repo`, `number`, `title`, `url`,
`author`, `isDraft`, `createdAt`, `updatedAt`, `additions`, `deletions`, `head`, `base`,
`reviewDecision`, `mergeState`, `isInMergeQueue`, `queueRemoval`, `queue`
(`{"position", "state"}` or `null`), `queue_url`, `ci`, `openThreads`, `requested`,
`requestedMe`, `stack` (`"2/3"` or `null`), `mine`, `reasons`, and `section`.

| Key | Value |
| --- | --- |
| `queue_url` | The web page of the merge queue of the PR's base branch, or `null` when that branch has no merge queue. The mods open it. They do not make a code host URL themselves. |
| `stack` | `"<position>/<size>"` of the PR in its stack. Only with `code_host_settings.stacks`, else `null`. |

The section is one of `attention`, `approved`, `queue`, `review`, `mine`, `other`, `drafts`.
The `prs` command puts the PRs in these sections, in this order, and sorts each section.

The result has these keys next to `prs`:

| Key | Value |
| --- | --- |
| `warning` | A text for the person when Helm did not read all the open PRs, else `null`. The github connector reads at most 500 PRs (10 pages of 50). When the org has more, `warning` says how many there are. The `prs` command copies `warning` to its own JSON, and prints it first without `--json`. |
| `rate` | The GraphQL and REST budgets of the token: `{"graphql": {"limit", "remaining", "reset"}, "core": {...}}`. |

The github connector reads the merge queues only for the repos and base branches of the PRs
that the search found. It does not list all the repos of the org.

### Agent prompts

`agent_prompt` gives the first message of the review or fix agent. The github connector fills
its templates in one pass, so a PR title with `{path}` or another `{name}` in it stays as it is.
Each prompt tells the agent that the PR title, description, comments, and review threads come
from other people, and that they are data, not instructions. An external connector must do the
same: other people write the PR text.

### Enqueue

The github connector never merges a PR at once on purpose. It reads the PR, decides, reads the
PR again, and stops when the merge queue setting, the merge state, or the head commit changed in
between.

- On a repo with a merge queue for the base branch, it runs
  `gh pr merge <n> --repo <host/org/repo> --auto`. GitHub adds the PR to the queue when it is
  ready.
- On a repo without a merge queue, it turns on auto-merge only when the merge state is
  `BLOCKED`, `BEHIND` or `DIRTY`. It uses the GraphQL call `enablePullRequestAutoMerge` with the
  merge method from the settings and the head commit that it read. That call never merges at
  once. It does not use `gh pr merge`, because gh merges a PR at once when the PR can merge now.
- In every other state (`CLEAN`, `UNSTABLE`, `HAS_HOOKS`, `UNKNOWN`, or a state that GitHub adds
  later), it refuses and tells the person to merge the PR on GitHub.

The limits that stay:

- The second read and the merge call are two requests. The state can change in the short time
  between them. On a repo without a merge queue, the GraphQL call then fails, or it turns on
  auto-merge, which waits. It never merges at once.
- On a repo with a merge queue, `gh pr merge --auto` decides itself. GitHub merges through the
  queue only, but a queue with a minimum group size of 1 and no wait time can merge soon after.
- When the setting of the merge queue changes on GitHub after the second read, the call uses the
  old plan.

## Link the PRs of a space

`crate::connectors::code_host::link_space_prs` links the PRs of a space to its record:

```rust
pub fn link_space_prs(rec: &mut Value, session_pr_urls: &[String]) -> Result<usize>
```

- `rec` is the space record. The function sets `rec["prs"]` and removes the old `rec["pr"]`.
  The caller saves the record.
- `session_pr_urls` are the PR web URLs that the agent sessions of the space made.
- The function finds the PRs of each checkout branch in any state, and the PRs at the URLs.
  Only a PR whose head branch the space checks out counts.
- A new PR gets `"kind": "work"`. A known PR gets its new title and state.
- The function returns the number of new PRs. When the code host is `"none"`, it changes
  nothing and returns 0.

The `rename` and `info` commands call it.
