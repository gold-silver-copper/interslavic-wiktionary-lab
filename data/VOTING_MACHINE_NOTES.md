# Notes from van Steenbergen's "voting machine" (the reference implementation)

Source: <http://steen.free.fr/interslavic/voting_machine.html> + `../scripts/transliteration.js`.
This is the original tool that generates Interslavic words by the prototype/consensus
method. Its JavaScript is the reference our `consensus.rs` reproduces. Extracted and
analyzed to look for improvements.

## Its algorithm (from `vote()` and the per-language `X_slo()` functions)

1. **Per-language normalization** (`ru_slo`, `pl_slo`, `cz_slo`, …): each language's word is
   normalized toward a common phonemic/etymological form. Cyrillic goes through `cyr_lat`;
   Latin scripts get regex rules. Highlights:
   - **Polish** (`pl_slo`): `cz→č sz→š ż→ž`, `rz→r` (`rj` before a vowel), **`ci→ti dzi→di`**
     and **`ć→t dź→d ś→s ź→z`** (de-palatalize soft dentals to the etymological stop),
     `ia→ja ie→e io→jo iu→ju`, **`ą/ę→u`** (nasals collapsed for comparison), **`l→lj ł→l`**,
     `ch→h w→v x→ks ó→o`, **`y→i`**, `ń→nj`.
   - **Czech** (`cz_slo`): long vowels shortened, **`ou→u`**, `ch→h w→v`, `ľ→lj ň→nj`,
     `ř([aou])→rj / ř→r`, `ť([aou])→tj / ť→t`, `ď([aou])→dj / ď→d`.
   - **Slovak** (`sk_slo`): like Czech, plus `i([aeou])→j$1` (iotation).
   - **Serbian** (`sr_slo`): `cyr_lat`, then `đ→dž ć→č`; auto-copies to HR/BS if blank.
2. **Split comma-separated variants** — each variant becomes its own vote with the
   language's full weight.
3. **Count** — group by *exact* normalized-string match, summing per-language vote weights
   and speaker millions.
4. **Sort** — winner = highest votes; ties broken by total **speaker population**.
5. Speaker weights (mln): RU 143.6, PL ~44, UA 37, CZ 10, BY 8.6, SK, SL, HR, SR, MK, BG…
   Vote weights vary by a **mode ("votetype" 0/1/2/3)** — different balancing profiles;
   RU is always weight 1, others scale with the mode.

## How it compares to our engine (`consensus.rs`)

- **Architecture matches.** Per-language normalize → weighted vote → pick winner → derive
  the flavored form. Our six-subgroup balancing corresponds to the reference's "balanced"
  voting mode (the reference also has a raw speaker-weighted mode, which the design criteria
  explicitly warn against because it lets Russian dominate — so our default is the better
  one).
- **Difference 1 — alignment key.** We vote on an aggressive *consonant-skeleton* key
  (vowels dropped), which is safe because within one meaning every form is cognate. The
  reference votes on *exact* normalized strings, so its per-language rules must be finer.
- **Difference 2 — surface.** We pick a representative language and "isvize" it, then (on
  linked words) derive the flavored form from Proto-Slavic. The reference's winning
  normalized string *is* the output.

## Ports tested against our benchmark — all REGRESSED (kept for the record)

| Ported idea | Result vs production (34.72% / 41.48%) |
|---|---|
| Polish/Czech/Slovak de-palatalization (`ci→ti`, `ć→t`, `dź→d`, `ou→u`) | 34.58% / 40.80% — **regress** |
| Soft-dental only (`ć→t`, `dź→d`) | 34.58% / 40.94% — **regress** |
| All-variants voting (each comma-variant votes, like the reference) | 33.44% / 39.91% — **regress** |

**Why they don't transfer:** the reference's rules are tuned to *its* pipeline — exact-match
voting on human-cleaned single-word input. Our downstream is different (aggressive
consonant-key alignment + representative isvization + Proto-Slavic derivation), and our
input is messy multi-variant dictionary cells. The de-palatalization changes the surface
representative for the worse; the extra variants add noise our representative-selection
can't filter as a human would.

**Conclusion:** the voting machine *validates* our architecture but offers no drop-in win.
The real remaining levers stay the ones in [`docs/history/IMPROVEMENT_PROMPT_V3.md`](../docs/history/IMPROVEMENT_PROMPT_V3.md) §C/§D (root modeling,
link coverage) — not the per-language normalization tables.
