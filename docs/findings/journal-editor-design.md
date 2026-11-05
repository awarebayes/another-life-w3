# Journal editor: design, prototype, gate J

Date 2026-11-04. Builds on `quest-state.md` (where quest state lives). Code: `crates/w3save/src/journal.rs`,
`crates/w3al-core/src/journal_effect.rs`, `crates/w3al-cli/src/journal_cmd.rs`, tests `crates/w3save/tests/journal_fixtures.rs`.
Labels: **[proven]** = checked on real saves by a test, **[hypothesis]** = needs the game (gate J, `C:\w3-out\gate_J\README.txt`).

## 0. New facts found while building it

| Finding | Evidence | Consequence |
| --- | --- | --- |
| Table index of every node = its **post-order rank** (children first, then the node, file order) | 122 of 122 files (101 Nexus + 21 own, every game version), `tests/tokens_fixtures.rs` | Adding nodes is safe to number: renumber the whole tree in post-order and the table is what the game would write. `Document::renumber_post_order`, `indices_are_post_order`; `rebase` now writes a table whose length follows the tree. |
| Entry layout is fixed: `SJournalEntryStatus { path { (guid, resource, flags) x 1..5 }, status, JEntryUnread }`, all `VL` | strict `journal::read` on 101 of 101 5.x saves, 58,632 entries | New entries can be built from data (no donor node needed). Depth 5 exists in late-game saves (181 entries); 4 in early ones. |
| `flags` = 0 on every step but the last, 1 on the last | 58,632 of 58,632 entries | TOML does not carry flags; the code derives and checks them. |
| No duplicate entry keys (path GUIDs) in any save | 101 saves | An entry is addressed by its GUID path. |
| The game only appends entries and never reorders them | 44 distinct consecutive pairs (88 with the duplicate set) | "append at the end" is what the game itself does. |
| `JObjectiveCounters` are in **hash order**: a new counter lands anywhere (21 -> 22: slot 27 of 45); the old ones keep their order, except 4 of 44 pairs where the game re-hashed (every counter moved) | pair test + replay test | Counter position is not semantic (probably). A new counter can be inserted at the donor's position (`at`) or appended. |
| The game's NM name table is **append-only** between saves: B's names = A's names + new ones at the end in 43 of 44 pairs (06 -> 07 differs from name 2,432 on) | probe over lukwraith 01..45 | A missing name (e.g. `JS_Success` in save 01, `JS_Failed` early) can be added by appending to `MANU`; no index anywhere changes. `Document::intern_name`. |
| `CWitcherJournalManager` also holds `JHuntingClues`, `JMonsterKnown`, `JEntryAdvancedInfo` | 14 of 88 replayed pairs change only those | Not modelled; `journal-diff` reports them; not needed for quest status. |

## 1. Offline proof (the strongest one we can get without the game)

`apply(diff(A, B))` on A, compared with B (`tests/journal_fixtures.rs`):

* **lukwraith 21 -> 22 (mq0005 completed): `CJournalManager` (44,979 bytes) AND `CWitcherJournalManager` (1,950 bytes) are
  byte-identical to the game's own save 22**; every other top-level block is still save 21's; the table grows by 69 entries (6 entries
  + 1 counter), re-parses, post-order holds. Numbers match `quest-state.md`: 5 flips Active -> Success (2 bytes each, unread unchanged),
  6 appended (2 mq0005 end-state + 4 tutorial), Size 208 -> 214, counter `753019eb` added, tracked quest `0f3305c0` -> `7c198a76`,
  highlighted objective `b1b2703e` -> `6b9fcac1`.
* All 44 consecutive pairs: the decoded quest log of the replay equals B's (counters compared as a set where the game re-hashed);
  `CJournalManager` byte-exact on 43 of 44 (the 44th is 01 -> 02, where `JS_Success` had to be appended and got index 1,817 instead of
  the game's 1,837: same name, other number); `CWitcherJournalManager` byte-exact on 34, 7 differ only in unmodelled side tables, the rest
  only in counter order after a re-hash.
* The gate files J1/J2 themselves: both journal blocks byte-identical to save 22 (re-checked on the written files).

What this does NOT prove: that the game reads these blocks as the quest log on load (it might rebuild them), that a stale
`questSystem` frontier (still waiting at the Napp decision) stays quiet, or that our NM-append is accepted. That is gate J.

## 2. API (`w3save::journal`)

```rust
pub fn read(doc: &Document) -> Result<Journal>;              // strict; any other shape is Error::Unverified
pub struct Journal { size, entries: Vec<Entry>, counters_size, counters: Vec<Counter>, tracked_quest, highlighted_objective }
pub struct Entry { path: Vec<Step>, status: String /* NM name */, unread: bool }   // key() = path GUIDs, label() = "minorquests/mq0005dwarfsmith/f02a0417"
pub struct Step { guid: [u8; 16], resource: String, flags: u8 }

pub fn diff(a: &Journal, b: &Journal) -> JournalDiff;        // changed / added / removed by key; append_only; counters; tracking
impl JournalDiff { pub fn to_edit(&self) -> Result<JournalEdit> }   // refuses removals and path-detail changes

pub struct JournalEdit {
    status: Vec<StatusChange { key, from: Option<String>, to, unread: Option<bool> }>,
    append: Vec<Entry>,                  // "ensure": appended at the end if missing, else its status/unread set
    counters: Vec<CounterEdit { guid, count, at: Option<usize> }>,   // set, or insert (at = donor position) / append
    tracked_quest: Option<Guid>, highlighted_objective: Option<Guid>,
    allow_new_names: bool,               // append a missing JS_* name to MANU (hypothesis; off by default)
}
pub fn apply(doc: &mut Document, edit: &JournalEdit) -> Result<JournalReport>;
pub fn unmodelled_changes(a: &Document, b: &Document) -> Vec<String>;
```

`apply` is all-or-nothing: it works on a clone, checks `Size == entries`, post-order numbering (only when nodes are added), resolves
every name and position first, mutates, renumbers, `rebase`s, re-reads the journal and checks each requested change and that nothing
else moved, then replaces the document. `from` guards against a template in an unexpected state (a template that is already `to` is a
no-op). New nodes are built from NM indices of the template (`SJournalEntryStatus`, `path`, `guid`, `CGUID`, `resource`, `String`,
`flags`, `Uint8`, `status`, `EJournalStatus`, `JEntryUnread`, `Bool`, `JObjectiveCounter`, `count`, `Int32`); a missing schema name is
refused. Supporting changes in `Document`: `renumber_post_order`, `indices_are_post_order`, `intern_name`, `name_index`,
`top_nodes_mut`; `rebase` accepts a table that grew. `Edits`/`Document::apply` are unchanged and still refuse journal edits.

CLI:
* `w3al journal <save> [--status S] [--grep text] [--counters]` - list entries (index, status, unread, depth, label, GUID path), counts
  by status, tracked quest and objective with their entry labels.
* `w3al journal-diff A B [--grep text] [--key name]` - flips/appends/removals, counters, tracking, unmodelled side tables, missing
  status names, then a ready-to-paste `[effects.<key>]` TOML block with `source = "save diff <A stem> -> <B stem>"`, `placeholder = false`.
* `w3al journal-set --template T [--set ENTRY=STATUS] [--append-from DONOR:ENTRY] [--from-toml F] [--counter GUID=N] [--track G]
  [--highlight G] [--fact NAME=V] [--allow-new-names] [--stem S] --out-dir D` - new file only (`create_new`, sidecars copied, default
  `ManualSave_` stem), then re-reads it: parse, table, post-order, `check-save`, re-applying the edit changes nothing, only the asked
  facts changed.

## 3. Questbook schema (implemented in `w3al-core`, accepted by lint, refused by export until gate J)

Entries are GUID paths that cannot be derived from quest names, so they are captured, never typed by hand:

```toml
[effects.done]
source = "save diff ManualSave_fd3e1_7ea4d800_3f4cf08 -> ManualSave_8559a_7ea4d800_4038706 (w3al journal-diff)"
placeholder = false
facts = { mq0005_complete = 1 }

[[effects.done.journal_status]]          # an entry the template already has
label = "minorquests/mq0005dwarfsmith"   # for reviewers only
path = ["67e5afe3ceb5c148a9cde5c4a99c7a84", "0f3305c02a250d4aa75c6fba97048763"]
from = "JS_Active"                       # template must be this (or already `to`)
to = "JS_Success"

[[effects.done.journal_append]]          # ensure it exists (appended at the end when missing)
label = "minorquests/mq0005dwarfsmith/f02a0417/753019eb"
status = "JS_Success"
unread = true
steps = [
  { guid = "67e5afe3ceb5c148a9cde5c4a99c7a84", resource = 'gameplay\journal\quests\minorquests.journal' },
  { guid = "0f3305c02a250d4aa75c6fba97048763", resource = 'gameplay\journal\quests\mq0005dwarfsmith.journal' },
  { guid = "f02a04176a249e4db9ce2c27f4810491", resource = "" },
  { guid = "753019eba7b3cf48ae3f0b4af472783c", resource = "" },
]

[[effects.done.journal_counters]]
guid = "753019eba7b3cf48ae3f0b4af472783c"
count = 0
at = 27        # optional: donor position (hash order); only meaningful on the capture's own 'before' save

[effects.done.journal_track]
quest = "7c198a768adc3046b34b5a730b5b53c9"
objective = "6b9fcac1d1bc9946990bdc429ea8978c"
```

Capture workflow (content authors):
1. Find a pair of real saves around the event: last save before the decision / completion, first save after (a playthrough with
   consecutive manual saves, like lukwraith's, gives these for free).
2. `w3al journal-diff before.sav after.sav --grep mq0005 --key done` (grep drops unrelated tutorial/bestiary entries and counters; tracking
   is left to the author because it depends on what the player does next).
3. `w3al facts before.sav --diff after.sav --grep mq0005` for the facts of the same effect (dialogue-id facts `NNNN_n_n` with expiry -2 are
   left out).
4. Paste both into the quest file, keep `source` (the save names), `placeholder = false`. `w3al lint` validates GUIDs, statuses and depth.
5. Choice branches: capture each branch from a pair that took that branch; the quest-level flips are usually shared, the branch-specific
   objective entries differ.

## 4. Template strategy

| Template state for quest X | Completed | Started | Skipped |
| --- | --- | --- | --- |
| **mid-quest** (entries Active, quest graph waiting at the last decision; lukwraith 21) | flips + few appends (gate J) | nothing | impossible (entries exist) |
| **before the quest** (no entries) | append the whole entry set (Case B); quest graph still has the quest's start triggers waiting | append first entries; risky | nothing |
| **after the quest** (entries Success) | nothing | impossible without removals | impossible without removals |

Recommendation:
* Act templates should be taken **with the act's choice-bearing quests started and parked at their decision** (the game itself saved in
  that state, so the entries exist as Active and only status flips + end-state appends are needed). That is the robust case: `from`
  checks it, `journal_append` is "ensure" so a template that already has an end-state entry is fine.
* Appending a whole quest into a template that never started it works at the byte level (proven: the entries are pure data) but leaves the
  quest graph able to start the quest again; treat it as a fallback that needs its own in-game test.
* "Skipped" quests must come from templates where the quest never started (the game never removes entries; removal is unsupported).
* The tracked quest should be re-pointed to a still-active quest when the tracked one is completed (the game does it); the file-name `a`
  then follows the tracked quest (J1/J2 use `8559a`, the key of `7c198a76` q001yennsearch, see `save-name-ids.md`).

## 5. What cannot be done safely (yet)

* **Status names missing from the template's NM table.** Early saves lack `JS_Success` (save 01) and all lukwraith saves lack `JS_Failed`.
  Assessment: easy technically. The NM table is append-only in the game's own saves, an appended name changes no existing index,
  `MANU` grows and `rebase` moves the tail (implemented: `Document::intern_name`, `--allow-new-names`, unit-tested, replayed on 01 -> 02).
  No index rewrite is needed. Risk: whether the game accepts a name we appended is untested (low risk: it is the same operation the game
  does, but it is native code). Not used in gate J.
* **Quest-level `JS_Failed`:** never seen on a quest-level entry in 101 saves (113 `JS_Failed` entries, all groups/objectives). Failing a quest
  through the journal is unproven in every sense.
* **Removing entries** (un-starting a quest): never seen in game data; unsupported.
* **The quest graph** (`questSystem`) and world effects (`community` spawn sets) are not edited. A completed quest's scene blocks stay in the
  frontier of a mid-quest template; whether they re-fire is the key question of gate J.
* **Side tables** `JHuntingClues`, `JMonsterKnown`, `JEntryAdvancedInfo` are not modelled.
* **Counter position**: we insert at the donor's position or append; the game's hash function is unknown.
* `Document::apply` (and so the app's export) keeps refusing journal effects until gate J passes.
