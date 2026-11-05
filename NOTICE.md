# Notice

## Not affiliated

Another Life for The Witcher 3 is an unofficial fan project. It is not affiliated with,
endorsed by or sponsored by CD PROJEKT S.A. or CD PROJEKT RED. *The Witcher*, *The Witcher 3:
Wild Hunt* and related names, characters and logos are trademarks or copyrights of
CD PROJEKT S.A. and are used here only to say what the tool works with.

## No game files

This repository and its releases contain no game files, no game text, no extracted data lists
and no saves. The tool reads the player's own install and the player's own save files on the
player's machine. Template saves used by the installer are built locally from the author's
own saves and are not distributed through git.

## Illustrations are linked, not stored

Quest and act illustrations are **linked**, not copied: the app fetches public image URLs
from the Witcher Wiki on Fandom (`static.wikia.nocookie.net`) at run time and shows a credit
and a link to the wiki page. Most of those images are CD PROJEKT RED material that the wiki
hosts; they are not licensed through this project. Online images can be turned off in the
app, and the app works without them. No image file is stored in the repository or installer.

## Third-party code and references

- The save container and variable-token layouts were first described by
  [Atvaark/W3SavegameEditor](https://github.com/Atvaark/W3SavegameEditor) (MIT licence).
  `crates/w3save` (`payload.rs`, `token.rs`) follows its structure and was checked against
  real saves; the credit is also in the source comments.
- The mustiny8402 save recovery tool is a reference for understanding the format only;
  no code is taken from it.
- Rust crates are used under their own licences (see `Cargo.lock`); the Tauri shell is
  MIT OR Apache-2.0.

## Licences of this repository

- Code: MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`).
- Questbook content (`questbook/`): CC BY-SA 4.0 (`questbook/LICENSE`).
