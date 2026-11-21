#!/usr/bin/env python3
"""Generates the chord progression library: 100 progressions for each of the drum rack's genres.

    python3 scripts/chords/make_progressions.py > crates/audiozero-engine/src/progressions.json

Each entry is {"id", "genre", "name", "tag", "mood", "chords"}. `chords` is roman numerals
relative to the key with a length in beats after the colon ("i:4 bVI:4 bIII:2 bVII:2"), read by
`Numeral::parse` in chords.rs. Numerals ending in a quality are read the way chords.rs reads
them: "ii7", "Vmaj7", "V7", "vii7b5", "IVsus4", "bVII", "Imaj9".

What's in it: the classic progressions of each mood, then progressions grown from the usual
chord-to-chord moves with each genre's own colour (sevenths and ninths in lo-fi and jazz, plain
minor triads in trap and dubstep, sus chords in techno), its own chord counts and lengths, and its
own share of major and minor. Everything is seeded, so the file is the same every time, and
every progression in a genre is different from the rest.
"""
import json, random

GENRES = ["techno", "house", "dubstep", "hiphop", "lofi", "trap", "dnb", "halftime", "jazz"]
PER_GENRE = 100

# ---- the usual moves, by mood (the same tables as chords.rs, plus a few more)
MAJOR = {
    "I": [("IV", 3), ("V", 3), ("vi", 3), ("ii", 2), ("iii", 1), ("bVII", 1)],
    "ii": [("V", 5), ("IV", 1), ("viidim", 1), ("I", 1)],
    "iii": [("vi", 3), ("IV", 3), ("ii", 1)],
    "IV": [("V", 4), ("I", 3), ("ii", 2), ("vi", 1), ("iv", 1)],
    "V": [("I", 4), ("vi", 3), ("IV", 2)],
    "vi": [("IV", 3), ("ii", 3), ("V", 2), ("iii", 1)],
    "viidim": [("I", 3), ("iii", 1)],
    "bVII": [("IV", 3), ("I", 3)],
    "iv": [("I", 3), ("V", 1)],
}
MINOR = {
    "i": [("bVI", 3), ("iv", 3), ("bVII", 3), ("v", 1), ("bIII", 2), ("V", 1)],
    "bIII": [("bVII", 3), ("iv", 2), ("bVI", 2), ("i", 2)],
    "iv": [("bVII", 3), ("i", 2), ("V", 2), ("bVI", 1)],
    "v": [("i", 3), ("bVI", 2), ("bIII", 1)],
    "V": [("i", 5), ("bVI", 1)],
    "bVI": [("bVII", 3), ("bIII", 2), ("i", 2), ("iv", 1), ("V", 1)],
    "bVII": [("i", 4), ("bIII", 2), ("bVI", 1), ("iv", 1)],
}
START = {"major": [("I", 14), ("vi", 4), ("IV", 2)], "minor": [("i", 16), ("bVI", 2), ("iv", 2)]}
END = {"major": [("V", 9), ("IV", 3), ("I", 5), ("vi", 3)],
       "minor": [("bVII", 8), ("V", 5), ("v", 3), ("bVI", 4)]}

CLASSIC = {
    "major": [
        "I V vi IV", "I vi IV V", "vi IV I V", "I IV V IV", "I IV vi V", "I V IV I", "IV I V vi",
        "I iii IV V", "I vi ii V", "I IV I V", "vi ii V I", "I iii vi IV", "IV V iii vi",
        "I V vi iii IV I IV V", "I IV ii V", "I vi iii IV", "ii V I vi", "I V vi IV I V IV V",
        "vi V IV V", "I IV V I", "IV I vi V", "I ii iii IV", "I bVII IV I", "IV V vi",
    ],
    "minor": [
        "i bVI bIII bVII", "i bVII bVI V", "i iv v i", "i bVI iv V", "i bIII bVII iv",
        "i iv bVII bIII", "i bVII bVI bVII", "i v bVI iv", "bVI bVII i i", "i bVII", "i bVI",
        "i iv i V", "i bVI bVII i", "i bIII iv bVII", "i bVII bVI bVII i bVII bVI V",
        "bVI bVII i v", "i iv bVI bVII", "i bVI bIII bVII i bVI iv V", "iv i bVII bVI",
        "i V bVI bVII", "i bIII bVI bVII", "i iv V i", "bVII bVI i i",
    ],
}

# ---- what each genre sounds like
#   minor:   share of minor progressions
#   p7, p9:  how often a chord becomes a seventh, and a seventh a ninth
#   psus:    how often a triad becomes a sus chord
#   shapes:  (number of chords, bars) with how many beats each lasts
GENRE = {
    "techno":   dict(minor=0.9, p7=0.05, p9=0.0, psus=0.2, shapes=["2x4", "2x8", "4x4", "4x8", "3x4", "1x4"], classic=0.25),
    "house":    dict(minor=0.5, p7=0.55, p9=0.2, psus=0.05, shapes=["4x4", "4x8", "2x4", "6h4", "6x4", "8x8"], classic=0.3),
    "dubstep":  dict(minor=0.9, p7=0.05, p9=0.0, psus=0.05, shapes=["2x4", "4x4", "4x8", "2x8", "3x4"], classic=0.2),
    "hiphop":   dict(minor=0.6, p7=0.6, p9=0.25, psus=0.0, shapes=["4x4", "2x4", "6h4", "4x8", "3x4", "6x4"], classic=0.25),
    "lofi":     dict(minor=0.45, p7=0.85, p9=0.4, psus=0.02, shapes=["4x4", "6h4", "6x4", "8x8", "4x8", "3x4"], classic=0.2),
    "trap":     dict(minor=0.92, p7=0.1, p9=0.0, psus=0.0, shapes=["2x4", "4x4", "4x8", "3x4", "2x8"], classic=0.25),
    "dnb":      dict(minor=0.8, p7=0.45, p9=0.2, psus=0.03, shapes=["4x4", "4x8", "6x4", "2x4", "6h4", "8x8"], classic=0.2),
    "halftime": dict(minor=0.7, p7=0.4, p9=0.1, psus=0.03, shapes=["4x8", "4x4", "2x4", "3x4", "2x8", "6x4"], classic=0.2),
    "jazz":     dict(minor=0.45, p7=0.98, p9=0.35, psus=0.0, shapes=["4x4", "6h4", "6x4", "8x8", "3x4", "4x8"], classic=0.0),
}

def shape(code, rng):
    """(beats of each chord, how many chords) for a shape code: "4x4" is 4 chords over 4 bars,
    "3x4" is 3 chords (one twice as long as the others) and "6h4" is 6 chords over 4 bars that
    change on the bar and the half bar."""
    n, kind = int(code[0]), code[1]
    total = int(code[2:]) * 4
    if kind == "h":
        return [4, 4, 2, 2, 2, 2], 6
    if n == 3:
        d = [total / 2, total / 4, total / 4]
        k = rng.randrange(3)
        return d[k:] + d[:k], 3
    if n == 6:
        return [total / 4, total / 8, total / 8, total / 4, total / 8, total / 8], 6
    return [total / n] * n, n

def pick(rng, options):
    total = sum(w for _, w in options)
    roll = rng.uniform(0, total)
    for name, w in options:
        roll -= w
        if roll <= 0:
            return name
    return options[-1][0]

def walk(rng, mood, count):
    table = MAJOR if mood == "major" else MINOR
    seq = [pick(rng, START[mood])]
    while len(seq) < count:
        last = len(seq) + 1 == count
        options = END[mood] if last else table.get(seq[-1], table["I" if mood == "major" else "i"])
        nxt = pick(rng, options)
        for _ in range(6):
            if nxt != seq[-1]:
                break
            nxt = pick(rng, table.get(seq[-1], options))
        seq.append(nxt)
    return seq

# ---- colour: triads to sevenths, ninths and sus chords
SEVENTH = {
    "I": "Imaj7", "IV": "IVmaj7", "ii": "ii7", "iii": "iii7", "vi": "vi7", "V": "V7",
    "viidim": "vii7b5", "i": "i7", "iv": "iv7", "v": "v7", "bIII": "bIIImaj7", "bVI": "bVImaj7",
    "bVII": "bVII7",
}
NINTH = {"Imaj7": "Imaj9", "IVmaj7": "IVmaj9", "ii7": "ii9", "iii7": "iii9", "vi7": "vi9", "V7": "V9",
         "i7": "i9", "iv7": "iv9", "v7": "v9", "bIIImaj7": "bIIImaj9", "bVImaj7": "bVImaj9", "bVII7": "bVII9"}
SUS = {"I": "Isus4", "IV": "IVsus2", "V": "Vsus4", "i": None, "bVII": "bVIIsus2", "bVI": "bVIsus2"}

def colour(rng, seq, g):
    out = []
    for t in seq:
        c = t
        if rng.random() < g["p7"] and t in SEVENTH:
            c = SEVENTH[t]
            if rng.random() < g["p9"] and c in NINTH:
                c = NINTH[c]
        elif rng.random() < g["psus"] and SUS.get(t):
            c = SUS[t]
        out.append(c)
    return out

# ---- jazz: the moves of standards
JAZZ = {
    "major": [
        "ii7 V7 Imaj7 Imaj7", "iii7 vi7 ii7 V7 Imaj7", "Imaj7 vi7 ii7 V7", "Imaj7 IVmaj7 iii7 vi7 ii7 V7 Imaj7 Imaj7",
        "ii7 V7 Imaj7 vi7", "Imaj7 bIII7 IVmaj7 bVII7", "Imaj7 ii7 iii7 IVmaj7", "IVmaj7 bVII7 Imaj7 Imaj7",
        "ii7 bII7 Imaj7 Imaj7", "Imaj7 vi7 IVmaj7 V7", "iii7 bIII7 ii7 V7", "Imaj7 bVII7 IVmaj7 Imaj7",
    ],
    "minor": [
        "ii7b5 V7 i7 i7", "i7 iv7 bVII7 bIIImaj7", "i7 bVImaj7 ii7b5 V7", "iv7 bVII7 bIIImaj7 bVImaj7",
        "ii7b5 V7 i7 iv7", "i7 bVII7 bVImaj7 V7", "i7 iv7 v7 i7", "bIIImaj7 bVImaj7 ii7b5 V7",
        "i7 i7 iv7 iv7", "i7 bIIImaj7 iv7 V7",
    ],
}

def jazz_walk(rng, mood, count):
    """Moves a fifth down, the way standards do, ending on a dominant or the tonic."""
    major = mood == "major"
    cycle = (["iii7", "vi7", "ii7", "V7", "Imaj7"] if major else ["bVImaj7", "ii7b5", "V7", "i7"])
    start = rng.randrange(len(cycle) - 1)
    seq = cycle[start:start + count]
    while len(seq) < count:
        seq.append(rng.choice(cycle))
    return seq

def build(genre):
    g = GENRE[genre]
    rng = random.Random(f"chords:{genre}")
    out, seen = [], set()

    def add(tokens, beats, tag, mood):
        key = tuple(zip(tokens, beats))
        if key in seen or not tokens or len(tokens) != len(beats):
            return False
        seen.add(key)
        out.append({"tokens": tokens, "beats": beats, "tag": tag, "mood": mood})
        return True

    # Classics first: a share of each genre's list, in its own colour.
    if g["classic"] > 0:
        want = int(PER_GENRE * g["classic"])
        pool = [(m, c) for m in ("major", "minor") for c in CLASSIC[m]]
        weights = [(1 - g["minor"]) if m == "major" else g["minor"] for m, _ in pool]
        tries = 0
        while sum(1 for o in out if o["tag"] == "Classic") < want and tries < 4000:
            tries += 1
            m, text = rng.choices(pool, weights)[0]
            tokens = text.split()
            if tries > 600 or rng.random() < 0.6:
                tokens = colour(rng, tokens, g)
            beats = {1: [16], 2: [8, 8], 3: [8, 4, 4], 4: [4] * 4, 6: [4, 2, 2, 4, 2, 2], 8: [4] * 8}.get(len(tokens), [4] * len(tokens))
            if rng.random() < 0.3 and len(tokens) == 4:
                beats = [8] * 4
            add(tokens, beats, "Classic", m)
    # Jazz standards' moves.
    if genre == "jazz":
        for m in ("major", "minor"):
            for text in JAZZ[m]:
                tokens = text.split()
                for beats in ([4] * len(tokens), [8] * len(tokens) if len(tokens) <= 4 else None):
                    if beats:
                        add(tokens, beats, "Standards", m)
    # The rest, grown from the usual moves.
    guard = 0
    while len(out) < PER_GENRE and guard < 200000:
        guard += 1
        mood = "minor" if rng.random() < g["minor"] else "major"
        code = rng.choice(g["shapes"])
        beats, count = shape(code, rng)
        count = min(count, len(beats))
        beats = beats[:count]
        if genre == "jazz" and rng.random() < 0.6:
            seq = jazz_walk(rng, mood, count)
            tag = "Circle of fifths"
        else:
            seq = walk(rng, mood, count)
            seq = colour(rng, seq, g)
            tag = None
        # A little extra: a secondary dominant or a borrowed chord now and then.
        if genre in ("jazz", "lofi", "hiphop", "house", "dnb") and rng.random() < 0.25 and count >= 3:
            i = rng.randrange(1, count - 1)
            nxt = seq[i + 1]
            lead = {"V7": "VI7", "ii7": "VI7", "vi7": "III7", "IVmaj7": "I7", "iii7": "VII7", "Imaj7": "V7"}.get(nxt)
            if lead:
                seq[i] = lead
                tag = tag or "Tension"
        add(seq, beats, tag, mood)
    assert len(out) >= PER_GENRE, (genre, len(out))
    out = out[:PER_GENRE]
    for o in out:
        o["tag"] = o["tag"] or label(o)
    return out

def label(o):
    t, mood = o["tokens"], o["mood"]
    rich = sum(1 for x in t if any(c in x for c in ("7", "9")))
    if len(set(t)) <= 2:
        return "Hypnotic"
    if any(x in ("VI7", "III7", "VII7", "I7", "vii7b5", "viidim", "bII7") for x in t):
        return "Tension"
    if rich >= len(t) - 1:
        if "ii7" in t and ("V7" in t or "V9" in t):
            return "Jazzy"
        return "Mellow" if mood == "major" else "Moody"
    if mood == "minor":
        return "Cinematic" if any(x.startswith(("bVI", "bIII")) for x in t) else "Dark"
    return "Uplifting" if t[0].startswith("I") else "Bright"

def main():
    entries = []
    for genre in GENRES:
        for n, o in enumerate(build(genre), 1):
            entries.append({
                "id": f"{genre}-{n:03d}",
                "genre": genre,
                "name": " – ".join(o["tokens"]),
                "tag": o["tag"],
                "mood": o["mood"],
                "chords": " ".join(f"{t}:{b:g}" for t, b in zip(o["tokens"], o["beats"])),
            })
    print(json.dumps(entries, indent=0, ensure_ascii=False))

main()
