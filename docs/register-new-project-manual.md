1) Start a Claude Code session in agent-knowledge-base-codex, then paste the template. (it cannot be codex session since everythings is setup in claude.md)


2) paste the template : Register a new project into the secondary brain:

- Path: C:\Users\quekm\Desktop\projects\<PROJECT-FOLDER>
- Agents I've used on it: <Claude Code | Codex | both | neither yet>

3) cross check is that one instructions (the required steps) append into the agents.md of that new registered added project 

4) the verification
Steps 1–3 get it set up. This is what proves it works like your two existing projects:

Check	Expected
Event count non-zero	brain status --project <id> — proves the backlog ingested
A query returns cited results	proves search and evidence work
Next Claude Code session in that folder	a claude-code / SessionStart delivery row appears
Next Codex session in that folder	a codex / brain_checkpoint row appears — proves AGENTS.md took effect

# One expectation to set
If you've never used Claude Code or Codex in that folder, registration produces an empty ledger — nothing to ingest. That's correct behaviour, not a failure. The hook will return empty until sessions accumulate. Your two current projects looked instant because they had months of history to ingest.