# Documentation

Where each kind of knowledge lives, and the conventions every document follows. Start here when you do not know which file owns a fact.

## Map

| I need to… | Read |
|---|---|
| Run, configure or deploy the bot | [`README.md`](../README.md) |
| Know the rules for changing the code | [`AGENTS.md`](../AGENTS.md) |
| Understand why the system is shaped this way (layers, runtimes, channels, accounts, storage, knowledge base, configuration vault) | [`architecture.md`](architecture.md) |
| Change the schema, write a query or set up test databases | [`database.md`](database.md) |
| Add or change an endpoint, an event or the authentication flow | [`api.md`](api.md) |
| Touch the assistant, a tool, the agent loop or the MCP server | [`llm.md`](llm.md) |
| Run or extend the local model server | [`../inference/README.md`](../inference/README.md) |
| Build or restyle the web UI | [`../Design.md`](../Design.md) |
| Pick up unfinished work or see what is settled | [`superpowers/HANDOFF.md`](superpowers/HANDOFF.md) |
| Cut over to production | [`superpowers/runbooks/`](superpowers/runbooks/) |
| Read the design behind a finished subsystem | [`superpowers/specs/`](superpowers/specs/) |
| Find the open findings of a review | [`superpowers/reviews/`](superpowers/reviews/) |
| See ideas nobody has committed to | [`brainstorming.md`](brainstorming.md) |

## Kinds of document

| Kind | Describes | Lifetime |
|---|---|---|
| Rules (`AGENTS.md`) | what to do and what never to do | updated when a rule changes |
| Reference (`architecture`, `database`, `api`, `llm`) | the system as the code makes it | always current; changed in the same commit as the code |
| State (`HANDOFF.md`) | what is done, open and settled | rewritten as work lands |
| Runbook | an operator procedure with rollback | kept until the procedure is obsolete |
| Spec, plan, review | one decision or audit at a point in time | deleted when fully shipped; git history keeps it |

## Conventions

1. **One owner per fact.** Each fact lives in exactly one document and every other place links to it. The command list that gates a change is `AGENTS.md` §8; the endpoint list is `backend/openapi.json`; the step-up routes are the `SENSITIVE` list in `interface/http/mod.rs`; environment variables are `.env.example`. Copy none of them.
2. **Describe the code as it is.** A reference document states current behaviour and the reason for it. History ("used to", "was removed in") is kept only where it prevents a repeat mistake. When a decision changes, update the owning document in the same change.
3. **Cite code, not review IDs.** Name the file, type or test (`interface/channels/inbound.rs`, `only_application_publishes_to_the_outbox`). Finding IDs such as `H4` or `R71` mean nothing once the review is deleted; they belong only in the review itself and in a HANDOFF ruling that points at it.
4. **Do not record numbers that rot.** No test counts, line counts or "N endpoints". State the command that produces the number.
5. **Style.** Sentence-case headings. File and symbol names in backticks. Shell snippets in fenced `bash` blocks, without a `$` prompt. Relative links, never absolute paths. `README.md` files may use emoji headings; reference documents do not.
6. **Dated documents carry a status.** A spec or review opens with `Status: Proposed | Approved | Implemented (date)`. A ruling reads `Ruling: <decision> — <why> — <cost if wrong>` and goes in `HANDOFF.md` §5.
7. **File names.** Reference documents are lower-case nouns (`database.md`). Specs, plans and reviews are `YYYY-MM-DD-topic.md` under `docs/superpowers/{specs,plans,reviews}/`.

## Checking a document against the code

Before trusting or merging a documentation change, spot-check the claims that drift fastest:

- Paths and symbols exist: `git grep -n "<symbol>"`.
- Environment variable names appear in `.env.example` and `backend/src/config/`.
- Routes appear in `backend/openapi.json`; step-up routes in `SENSITIVE`.
- Commands match `.github/workflows/ci.yml` and `AGENTS.md` §8.
- Chat commands match each module's `commands()` in `backend/src/interface/chat/`.
- Every relative link resolves.
