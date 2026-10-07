# [2.0.77] cache-prototype-data never used when any mod has a "~" dependency

**What happens**
With `cache-prototype-data=true`, the data stage cache is written at every start ("Data stage cached in ...") but
never loaded ("Data stage cache not used.") as soon as one enabled mod has a dependency with the `~` prefix (does not
affect load order).

**To reproduce** (vanilla, no other mods)
1. `config.ini`: `[other] cache-prototype-data=true`
2. A mod `tilde-probe` with `info.json` dependencies `["base", "~ quality"]` and an empty `data.lua`.
3. Start the game twice: both starts log "Data stage cache not used."
4. Change the dependency to `"? quality"`: the second start logs "Data stage cache loaded in ...".

**Why** (found with the game's own PDB, function and class names below)
`ModDataCache::loadInternal` compares each cached package's dependencies with the current ones field by field,
including `ModDependency::affectsSorting` (offset 42). The cache file stores only `optional`, `hidden` and
`incompatible` for a dependency (three flag bytes after the version), and the deserialising constructor
`ModDependency::ModDependency(MapDeserialiser&)` leaves `affectsSorting` at its default `true`. A `~` dependency has
`affectsSorting == false`, so the compare always fails for that package and the cache is aborted.

**Fix** either write/read `affectsSorting` in the cache, or leave it out of the compare (the sorted mod list it
affects is already checked).

On a ~400-mod pack this is ~35 s of every start (the data stage); with the compare patched out locally the cache
loads in 1.5 s and the game reaches the main menu that much sooner.
