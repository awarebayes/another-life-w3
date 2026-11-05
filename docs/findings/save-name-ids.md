# What the three ids in `ManualSave_<a>_<b>_<c>` mean

Date: 2026-11-04. Method: 77 distinct real saves (lukwraith 45 consecutive saves, hosamnasr 9, imz 1, 16 older
saves from 2016-2023, the user's own 5.01 save and the files the game wrote after our gate A/B/C), read through the typed tree
(`Document::parse`) with a throwaway reader; file times from the extracted zips, `C:\w3-fixtures` and the file LIST of
`Documents\The Witcher 3\gamesaves` (names, sizes, times only; no save there was modified). Nothing was written to any save.

## Result in one line

`b` and `c` are the **local date and time of the save**, packed into two 32-bit words. `a` is an **opaque key of the
quest tracked at save time** (same quest, same `a`, across players and years). None of the three is a playthrough id.

## b and c: packed local date and time (OBSERVED)

The payload's `saveInfo` block holds, besides `magic_number`, `description`, `runtimeGUIDCounter`, `playthroughId` and
`count`, a history of `time` (u64) / `type` (u8) / `v` (u16 = 67) triples, one per save ever made in the playthrough, oldest
first, ascending. `count` equals the number of triples. A game re-save appends one or two triples (type 3 for a checkpoint, 2 for a
normal save): the user's 340-entry save became 342 entries after the game saved from it, and the original's last `time`
(`0x7ea4c4000103093a`) is still in the list.

For each save, the last `time` is `(hi << 32) | lo` and equals the file name's `b` and `c`:

| field | meaning | evidence |
| --- | --- | --- |
| `b` = `hi` | `year << 20 \| (month-1) << 15 \| (day-1) << 10`, low 10 bits 0 | `7ea50c00` = 2026-11-04, `7ea4c400` = 2026-10-18, `7e046000` = 2016-09-25. Decodes to the calendar date of the file time (in the player's own zone, so up to a day off against UTC) on all 76 name-bearing saves, 2016 to 2026; low 10 bits are 0 on all of them. |
| `c` = `lo` | `hour << 22 \| minute << 16 \| second << 10 \| millisecond` | `5a98ce3` = 22:41:35.227 and the file was written 2026-11-04 22:41:35 local; `5a639fc` = 22:38:14; `1030937` = 04:03:02 (file time 04:03:02). The user's saves are in UTC+4 (the machine's zone); lukwraith's `3b84b8d` = 14:56:18 against a zip time of 17:56:18 UTC, i.e. his own zone (UTC-3). |
| relation to payload | the name is taken 1 to 165 ms before the `time` entry (median 13 ms, 3 ms on the user's recent saves) | `lo - c` over 76 saves |

So the stamp is **local** time, not UTC, and it has millisecond resolution. Order of the two words is chronological, so
`(b, c)` compared as numbers sorts saves by time.

Earlier guesses in `docs/research.md` ("step 0x400", "monotonic counter") were this: 0x400 is one day in the `day` field
(so `b` steps by 0x400 per calendar day, and by 0x8000 at a month change), and `c` restarts every day.
`runtimeGUIDCounter`, `EngineTime`, `GameTime`, `magic_number`, `timerIdGenerator` and `count` are **not** related to `b` or `c`
(checked on the series; `EngineTime` and play time run at different rates and do not reset daily).

## a: tracked-quest key (OBSERVED correlation, function NOT decoded)

* Not a playthrough id: it changes inside one playthrough (lukwraith: `53db8`, `8559a`, `103e40`, `108885`, `fa7a8`, ... 13 values over 45 saves) and the same
  value shows up in different players' games and years (`55607` in 2018, 2023, 2026; `52586` in a 2018 QuickSave and the user's 2026 save; `8559a` for
  lukwraith and our gate B).
* Not found in the payload: no u32/u64 variable, raw byte pattern or ASCII form of `a` occurs in the decoded payload of the four saves tried.
* It is a function of `CWitcherJournalManager > JTrackedQuest > guid`. All 21 saves checked: same tracked-quest GUID gives the same `a`
  (e.g. `364d02db...` = `52586` in the user's 2026 save, its re-save and a 2018 QuickSave; `7c198a76...` = `8559a` in 3 saves), different GUID
  gives a different `a`. The user's gate A re-save has a different tracked quest (`762996ea...`) and so `52587`, while gate C's re-save kept `364d02db...` and `52586`.
  Not related to facts count, world, loading-screen video or `questSystem` (identical in the 52586 and 52587 saves).
* Related values often differ by 1 (`108885/6/7`, `53db8/9`, `fa7a8/9`, `52586/7`), so it looks like an index, not a hash. INFERRED, not shown.

Inference: the Load list takes the quest name from a lookup keyed by this `a`, which is why the same payload under an `AnotherLife_` name shows no quest.
Not proven: the game may instead only check that the name has the `<Prefix>_<hex>_<hex>_<hex>` shape.

## What the game requires (OBSERVED)

* It does **not** require `b`/`c` to match the payload: files named with invented ids (`7ea4c800_10309a1`, `7ea4cc00_10309b7`, both decode to plausible dates
  but not the save's real one) loaded, played and re-saved fine, and the Load list shows the quest name for them.
* It did not mind `a` being shared with the original (`52586` twice) or different (`52587` in the game's own re-save).
* Prefix `QuickSave_`, `CheckPoint_` carry the same three ids; only `ManualSave_` was tested for the Load-list label.

## Untested (open)

* Whether the Load list orders by the name's stamp, by the payload `time`, or by file modified time. Our stamp is built from UTC
  (std has no local offset), so for a UTC+4 player it is 4 h behind the game's own saves of the same moment. Test: export two saves
  from a template, drop them in `gamesaves` next to a fresh in-game save, check the order.
* Whether an unknown `a` (renamed template) still gets a quest label, and whether a wrong `a` shows a wrong quest name. The exporter keeps the template's `a`; fixed
  default `8559a` for templates without ids.
* Whether appending a `time`/`type`/`v` triple to `saveInfo` (and bumping `count`) would make the file self-consistent. Not needed for loading.
* A clash with the game's own later name is possible only with the same millisecond; the game's behaviour on a clash is unknown.
