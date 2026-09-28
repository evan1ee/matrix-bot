# Using Gitea

`matrix-bot` is developed on a self-hosted **Gitea** server. GitHub is only a mirror. This guide covers everyday use for people and coding agents: remotes, branches, pull requests, CI, and the rules agents must follow.

| | |
|---|---|
| Web UI | <https://git.mbm.one/evan/matrix-bot> |
| Git remote | `gitea` → `git@git.mbm.one:evan/matrix-bot.git` (SSH) |
| Server version | Gitea 1.27.3 |
| Default branch | `master` |
| Visibility | private: the web UI and the API need a signed-in account or a token |
| Mirror | `origin` → `https://github.com/evan1ee/matrix-bot.git` (GitHub; not where work happens) |

---

## 1. One-time setup

### SSH access

Gitea is reached over SSH. Add your public key under **Settings → SSH / GPG Keys** in the Gitea web UI, then check the connection:

```bash
ssh -T git@git.mbm.one        # expect a greeting naming your Gitea user
```

### Remotes

In a fresh clone:

```bash
git clone git@git.mbm.one:evan/matrix-bot.git
cd matrix-bot
git remote add origin https://github.com/evan1ee/matrix-bot.git   # optional: the GitHub mirror
```

In an older checkout that only has GitHub as `origin`:

```bash
git remote add gitea git@git.mbm.one:evan/matrix-bot.git
git fetch gitea
git branch -u gitea/master master
```

Check the result with `git remote -v`: `gitea` must be listed, and `origin` points to GitHub.

### Optional: `tea`, the Gitea CLI

Browser-free pull requests need `tea` and an access token.

1. Create a token under **Settings → Applications → Generate New Token**. Give it the repository scope, plus issues if you want to work with issues. **Never commit the token or paste it into a file in the repo.**
2. Install `tea`: a binary from <https://gitea.com/gitea/tea/releases>, or `go install code.gitea.io/tea@latest`.
3. Log in:

```bash
tea login add --name mbm --url https://git.mbm.one --token "$GITEA_TOKEN"
tea login default mbm
```

`gh` (the GitHub CLI) **does not work with Gitea**. It only sees the GitHub mirror.

## 2. Everyday workflow

### Branches

- Branch from an up-to-date `master` and use Conventional prefixes: `feat/…`, `fix/…`, `refactor/…`, `docs/…`, `ci/…`, `chore/…`.
- Commits follow Conventional Commits (`feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`, `ci:`; add `!` for a breaking change). See `AGENTS.md` → *Commits and PRs*.

```bash
git fetch gitea
git switch -c feat/my-change gitea/master
# … work, commit …
git push -u gitea feat/my-change          # -u: the branch now tracks Gitea
```

On the first push, Gitea prints a link to open a pull request:

```
remote: Create a new pull request for 'feat/my-change':
remote:   https://git.mbm.one/evan/matrix-bot/pulls/new/feat/my-change
```

### Keeping the GitHub mirror in sync

The mirror is updated by hand. After pushing to Gitea, push the same refs to `origin`:

```bash
git push origin feat/my-change
git push origin master            # after a merge on Gitea: fetch it first, then push master
```

The mirror could instead be kept in sync automatically: in the repository on Gitea, open **Settings → Repository → Mirror settings** and add a **push mirror**. It needs a GitHub token with repository write access. Until that is set up, keep pushing by hand.

### Pull requests

You can open a pull request in any of three ways:

- **Web UI:** open the `…/pulls/new/<branch>` link printed on push. Alternatively, go to **Pull Requests → New Pull Request** and choose `base: master` and `compare: <branch>`.
- **`tea`:**

  ```bash
  tea pr create --base master --head feat/my-change \
    --title "feat: short summary" --description "$(cat pr-body.md)"
  tea pr list                 # open PRs
  tea pr checkout 42          # check out PR #42 locally
  ```
- **REST API** (with a token):

  ```bash
  curl -sS -X POST https://git.mbm.one/api/v1/repos/evan/matrix-bot/pulls \
    -H "Authorization: token $GITEA_TOKEN" -H "Content-Type: application/json" \
    -d '{"base":"master","head":"feat/my-change","title":"feat: short summary","body":"…"}'
  ```

A PR description needs:
- a summary of **all** commits on the branch (`git log master..HEAD`), not just the last one;
- a test plan with checkboxes;
- the gates the reviewer or owner still has to run by hand.

To address review feedback, push more commits to the same branch; the PR updates itself. Don't force-push a branch someone else is reviewing unless the owner asks for it.

Merging is the repository owner's decision. Use the merge style the owner picks in the PR's merge button, then delete the branch.

## 3. CI (Gitea Actions)

- CI runs on **Gitea Actions** through a self-hosted `act_runner`. Gitea runs the workflows in `.github/workflows/`: it reads that directory whenever there is no `.gitea/workflows/`. Keep them there so the GitHub mirror stays compatible.
- **Triggers** (`ci.yml`): pushes to `master`, and pull requests into `master`. **Pushing a feature branch on its own does not run CI.** Open a PR to get a run.
- **Where to look:** the **Actions** tab of the repository, or the status checks at the bottom of the PR. Each job shows its log step by step.
- **Jobs:** `backend` (fmt, tests on PostgreSQL 18, OpenAPI drift, `cargo sqlx prepare --check`), `inference`, `frontend`. See `AGENTS.md` → *Testing and Verification*.
- **`act_runner` quirks.** These are why the workflows look the way they do; don't "simplify" them away:
  - Jobs run **inside a container**, so a service container answers to its **label** (for example `postgres`), not `127.0.0.1`. `docs/POSTGRES_MIGRATION.md` has the full story.
  - Job images must include **Node.js**, because JavaScript actions run on it and `act_runner` does not inject it.
  - The runner's default image assumes no build tools, so `ci.yml` installs `pkg-config`, `libssl-dev` and `cmake` explicitly.
- **Secrets** (for example `DOCKERHUB_USERNAME` / `DOCKERHUB_TOKEN` for `docker-image.yml`) live under **Settings → Actions → Secrets** on Gitea, which is separate from GitHub's secrets.
- `docker-image.yml` and `release.yml` run on pushes to `master` and on `v*` tags.

## 4. Issues, releases, tags

- **Issues** are handled in the web UI, with `tea issues`, or through the API `…/api/v1/repos/evan/matrix-bot/issues`.
- **Releases:** push an annotated tag (`git tag -a v1.2.3 -m "…" && git push gitea v1.2.3`). That triggers `release.yml`. Then write the release notes under **Releases** in the web UI, or with `tea releases create`.

## 5. Rules for coding agents

- **Gitea is the primary forge.** Push branches with `git push -u gitea <branch>` and open PRs on Gitea. Pushing to `origin` only mirrors the work.
- **Push, open a PR, merge, tag or release only when the user explicitly asks.** Each of these is a separate approval; one "push" does not approve a merge.
- **No `tea` or token available?** Don't ask the user for one, and don't try to create it. Push the branch and give the user the `…/pulls/new/<branch>` link Gitea printed.
- Never put a token in a command that gets logged or committed, in a file in the repo, or in a URL. Read it from an environment variable the user set.
- `gh` and GitHub-only tools (GitHub MCP servers, GitHub PR monitors) see **only the mirror**. Don't use them to read or change the state of Gitea PRs.
- A rejected push means the remote moved. Fetch and investigate. **Never force-push** unless the user explicitly asks.
- CI results live on Gitea (Actions tab or PR checks). If you can't reach them, ask the user for the result or reproduce the job locally with the commands in `AGENTS.md`.

## 6. Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `Permission denied (publickey)` | Your SSH key isn't on your Gitea account, or `ssh-agent` isn't loaded. Add the key (§1) and retest with `ssh -T git@git.mbm.one`. |
| API returns 404 for the repo | The repo is private and the request has no token. Add `-H "Authorization: token $GITEA_TOKEN"`. |
| No CI run after pushing a branch | Expected: `ci.yml` only runs for `master` and PRs into it. Open a PR. |
| CI database tests fail with `TEST_DATABASE_URL must point at a reachable server` | The job isn't running in its container, or it uses `127.0.0.1` instead of the service label. See §3. |
| `gh pr …` can't find the PR | `gh` only talks to GitHub. Use the Gitea UI, `tea` or the API. |
| The branch exists on Gitea but not on GitHub | The mirror is manual. Run `git push origin <branch>`. |
