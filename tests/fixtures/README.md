# Test fixtures

Fixtures are synthetic. Tests build them at run time in temp dirs and never
read the operator's own game data.

## Policy

- Never commit `level.dat`, world saves, resource packs, or anything copied out
  of a real Minecraft dir. This includes screenshots of real worlds.
- A fixture is a directory tree that only has the marker dirs the code checks,
  such as `versions`, `saves`, or `resourcepacks`. The dirs can be empty.
- Build fixtures in the test body with `tempfile` in Rust or a temp dir in the
  frontend tests. Delete them with the temp dir.
- File names in a fixture use the launcher layout the code scans, for example
  `.local/share/PrismLauncher/instances/one/.minecraft`.
- If a test needs a symlink, create it in the temp dir and point it at another
  temp dir.

## Checked-in files

This directory holds only policy docs. The fixture trees themselves stay out
of git. A test that needs a fixture on disk creates it.
