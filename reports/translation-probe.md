# Translation probe (mrzavec / Rogue 5.4.5 vocabulary)

**Denominator:** the committed 219-word probe `tools/translation-probe.txt` (Rogue-5.4.5 game vocabulary), each query walked through the documented `api/en` retry ladder by the same code path as `en --batch`. **Reported metric, not a gate:** coverage moves with data; this report keeps PRs honest without freezing them. **Leakage story:** the probe is an external vocabulary list; no accuracy path reads it.

Recorded baseline (V12): **147 verified / 44 generated-only / 28 miss**. This run: **147 / 44 / 28** — unchanged.

| Category | verified | generated-only | miss |
|---|---:|---:|---:|
| Potions | 13 | 1 | 0 |
| Scrolls | 17 | 1 | 0 |
| Rings | 10 | 3 | 1 |
| Sticks (wands/staffs) | 12 | 1 | 1 |
| Weapons | 5 | 4 | 0 |
| Armor | 8 | 0 | 0 |
| Monsters | 12 | 9 | 5 |
| Traps | 7 | 1 | 0 |
| Colors (potion appearances) | 19 | 5 | 3 |
| Stones (ring appearances) | 9 | 8 | 9 |
| Woods (staff materials) | 16 | 10 | 7 |
| Metals (wand materials) | 19 | 1 | 2 |
| **total** | **147** | **44** | **28** |

## Misses

- **Rings**: stealth
- **Sticks (wands/staffs)**: polymorph
- **Monsters**: aquator, hobgoblin, jabberwock, quagga, xeroc
- **Colors (potion appearances)**: cyan, ecru, plaid
- **Stones (ring appearances)**: alexandrite, carnelian, garnet, kryptonite, lapis lazuli, moonstone, peridot, stibiotantalite, taaffeite
- **Woods (staff materials)**: balsa, driftwood, ironwood, manzanita, rosewood, teak, zebrawood
- **Metals (wand materials)**: aluminum, pewter

## Generated-only (suggestions needing review)

- **Potions**: levitation
- **Scrolls**: teleportation
- **Rings**: dexterity, regeneration, teleportation
- **Sticks (wands/staffs)**: teleport to
- **Weapons**: mace, dagger, dart, shuriken
- **Monsters**: centaur, emu, griffin, kestrel, leprechaun, medusa, orc, wraith, yeti
- **Traps**: trapdoor
- **Colors (potion appearances)**: amber, crimson, magenta, topaz, vermilion
- **Stones (ring appearances)**: agate, germanium, jade, onyx, opal, sapphire, topaz, zircon
- **Woods (staff materials)**: banyan, cedar, cinnabar, cypress, eucalyptus, holly, mahogany, pecan, redwood, walnut
- **Metals (wand materials)**: electrum
