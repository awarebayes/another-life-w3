# Another Life for The Witcher 3: player guide

An unofficial fan tool, not affiliated with CD PROJEKT RED. It is early software: read the limits below.

## What it does

You read the story of The Witcher 3 as an illustrated text adventure and answer the big questions
(who lives, who joins you, who you love). When you finish, the app writes a new save file whose
world matches your answers. You load it in the game and carry on from there.

## Requirements

- The Witcher 3 Remastered, PC, patch 5.01.
- Close the game before you export or touch the saves folder.
- Windows (the installer is a Windows installer).

## Install

Run the installer (`Another Life_0.1.0_x64-setup.exe`) and start the app. The app is built from
source with `cargo tauri build`; a public download is planned.

## How to use it

1. Pick where your story stops. Quests before that point count as done, quests after it are left alone.
2. Answer the questions. Each option says plainly what it changes. If you do not care, keep the
   default.
3. Optional start kit (level, crowns, gear): planned, not available yet.
4. Check the Review screen, then press Export.
5. Start the game and open Load Game. The new save is named `ManualSave_...` and appears at the
   top of the list. Load it.

## Safety

- The app never overwrites a save. It writes a new file (plus small sidecar files) and refuses if the name exists.
- Still keep a backup of `Documents\The Witcher 3\gamesaves` before you try it.
- Steam Cloud may sync the folder and bring old saves back or restore deleted ones. If your list
  looks wrong, pause Cloud sync for The Witcher 3 while you test.

## Limits

- Unofficial; not affiliated with or endorsed by CD PROJEKT RED.
- Scenes that already happened are not replayed. The save only says they happened.
- Some decisions are left for you to play in the game (for example the Skellige succession and a few
  later choices). The question screen tells you when this applies.
- Only the quests in the questbook are covered; the rest of the story is still being written
  (Phase 3 and later). Some answers are still marked placeholder and will be refused on export.
- Tested on patch 5.01 only. Mods and other patches are not supported.
- Only story decisions are written: facts, quest log and quest graph. Rewards and map pins are not.

## Troubleshooting

- Export says a quest or effect is a placeholder: that choice is not finished yet. Skip that quest or pick another one.
- Export refuses with a message about the game folder: set the Game folder in the app (default is
  the Steam folder `C:\Program Files (x86)\Steam\steamapps\common\The Witcher 3`).
- The save does not show a quest name in the Load list: the file name must keep the `ManualSave_` shape; do not rename it.
- The game will not load the save or crashes: close the game, delete the new save (it is a copy), keep playing your old one, and report the issue with the save attached.
- Old saves reappeared: Steam Cloud, see Safety.
