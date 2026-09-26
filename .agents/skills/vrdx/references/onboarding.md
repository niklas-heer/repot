# Onboarding existing decisions

Use this once when a project adopts vrdx and already has decisions written down:
ADR folders such as `docs/adr`, `docs/decisions`, `adr/` or `doc/adr`, a decision
log section in a README, or design notes with explicit choices. `vrdx init`
reports the directories it recognises; look for the rest.

Goal: one vrdx record per past decision, so the collection is a consistent
baseline. Do not rewrite history.

1. List the candidate sources and confirm the set with the user before importing.
2. Create one record per source decision with `vrdx new --from-json - --json`:
   - `title`: the original title, shortened to 3–7 words if needed.
   - `decision`, `why`, `consequences`: condensed from the source. Keep wording
     that matters and link the source path instead of copying everything.
   - `date`: the original date when the source states it; otherwise omit it and
     say so in the report.
   - `status`: map the source status (`accepted`, `superseded`, `deprecated`,
     `rejected`). A source without a status stays `proposed` unless the user
     confirms it applies.
   - `tags`: the source's tags or one topic; reuse tags already in the collection.
3. Recreate relationships afterwards in the Markdown: supersession chains from
   the source, `related_to` for topical links. Use full IDs from `vrdx list --json`.
4. Run `vrdx validate --json`, then `vrdx fmt --check`.
5. Report the imported count, skipped sources with reasons, and any unknown dates
   or statuses. Leave the original files in place unless the user asks to remove
   them, and offer to add a pointer to the collection where they lived.

Never invent dates, authors, approvals or reasons the source does not state.
