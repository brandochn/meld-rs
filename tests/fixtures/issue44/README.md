# Issue #44 fixtures

Regression inputs for [issue #44](https://github.com/brandochn/meld-rs/issues/44),
derived from the session files the reporter (`hg42`) shared privately.

| Pair | Source | Lines (A / B) | Size (A + B) |
|---|---|---|---|
| `a-cut3.json` / `b-cut3.json` | `a-cut3.json` / `b-cut3.json` | 23 622 / 23 911 | 1.3 MB |
| `a-cut4.json` / `b-cut4.json` | `a-cut4.json` / `b-cut4.json` | 18 638 / 18 929 | 1.0 MB |

## Sanitisation

The originals contain personal browsing data. Every occurrence of these JSON
values is replaced by a random 16-hex-digit token:

| Key | Replacement |
|---|---|
| `"url"` | random 16-hex token |
| `"title"` | random 16-hex token |
| `"favIconUrl"` | random 16-hex token |
| `"lastAccessed"` | random 16-hex token (string, not a number) |

The mapping is a random bijection over the unique original values: equal
original values map to equal tokens and different values stay different, so the
diff sees exactly the same line-equality structure. The token is **not derived
from the original value**, so it cannot be recovered by guessing (unlike a
hash).

Everything else (keys, ids, indices, booleans, indentation, line order, line
count) is untouched. No `https://` URL, page title or base64 icon remains.
Ids and indices are internal tab identifiers, not personal data.

## Verification gate

The relabelling is injective, so the chunk list is unchanged by construction.
The historical check (original vs sanitised pair, after the v3 alignment
changes: Patience + Meld-style rebuild + balanced-Replace split) produced:

* cut3: 2 220 chunks, **byte-for-byte identical**
* cut4: 1 615 chunks, **byte-for-byte identical**

If the fixtures ever stop reproducing the reported behaviour, re-check this gate
first: a mismatch means the sanitisation changed the data.
